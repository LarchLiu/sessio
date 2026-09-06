use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use serde::{Deserialize, Serialize};
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
    pub name_zh: Option<String>,
    pub name_en: Option<String>,
    pub permissions: Vec<SessioAppPermission>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SessioAppPermission {
    Autoplay,
    ClipboardWrite,
    Downloads,
    Fullscreen,
    Gamepad,
    Modals,
    PointerLock,
    Popups,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SessioAppFileEncoding {
    #[default]
    Utf8,
    Base64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessioAppFileWriteRequest {
    app_directory_path: String,
    relative_path: String,
    data: String,
    #[serde(default)]
    encoding: SessioAppFileEncoding,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessioAppFileWriteResult {
    relative_path: String,
    bytes_written: usize,
}

const MAX_APP_FILE_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    name_zh: Option<String>,
    name_en: Option<String>,
    #[serde(default)]
    permissions: Vec<String>,
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
pub(crate) fn write_sessio_app_file(
    request: SessioAppFileWriteRequest,
) -> Result<SessioAppFileWriteResult, String> {
    let root = app_paths::apps_dir().map_err(|error| error.to_string())?;
    write_app_file_in(&root, request)
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
    let web_directory = directory.join("web");
    let content_directory = if web_directory.is_dir() {
        web_directory.as_path()
    } else {
        directory
    };
    let expected_name = format!("{slug}.html");
    let mut html_files = fs::read_dir(content_directory)
        .ok()
        .into_iter()
        .flatten()
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
    let logo_path = find_logo_path(content_directory).map(|path| canonical_or_original(&path));
    let config = read_app_config(&web_directory);

    Ok(SessioAppInfo {
        id: stable_app_id(&directory_path),
        slug,
        directory_path,
        html_path: html_path.map(|path| canonical_or_original(&path)),
        html_file_name,
        logo_path,
        name_zh: normalize_name(config.name_zh),
        name_en: normalize_name(config.name_en),
        permissions: normalize_permissions(config.permissions),
    })
}

fn read_app_config(web_directory: &Path) -> AppConfig {
    let path = web_directory.join("config.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

fn normalize_name(value: Option<String>) -> Option<String> {
    value.and_then(|name| {
        let trimmed = name.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn normalize_permissions(values: Vec<String>) -> Vec<SessioAppPermission> {
    values
        .into_iter()
        .fold(Vec::new(), |mut permissions, value| {
            let permission = match value.trim() {
                "autoplay" => Some(SessioAppPermission::Autoplay),
                "clipboardWrite" => Some(SessioAppPermission::ClipboardWrite),
                "downloads" => Some(SessioAppPermission::Downloads),
                "fullscreen" => Some(SessioAppPermission::Fullscreen),
                "gamepad" => Some(SessioAppPermission::Gamepad),
                "modals" => Some(SessioAppPermission::Modals),
                "pointerLock" => Some(SessioAppPermission::PointerLock),
                "popups" => Some(SessioAppPermission::Popups),
                _ => None,
            };
            if let Some(permission) = permission.filter(|item| !permissions.contains(item)) {
                permissions.push(permission);
            }
            permissions
        })
}

fn write_app_file_in(
    root: &Path,
    request: SessioAppFileWriteRequest,
) -> Result<SessioAppFileWriteResult, String> {
    let root = fs::canonicalize(root).map_err(|error| format!("Invalid apps root: {error}"))?;
    let app_directory = fs::canonicalize(&request.app_directory_path)
        .map_err(|error| format!("Invalid app directory: {error}"))?;
    if app_directory.parent() != Some(root.as_path()) {
        return Err("App directory must be installed directly under the Sessio apps root".into());
    }

    let web_directory = fs::canonicalize(app_directory.join("web"))
        .map_err(|error| format!("Invalid app web directory: {error}"))?;
    if !web_directory.is_dir() || web_directory.parent() != Some(app_directory.as_path()) {
        return Err("App web directory must be a real directory inside the app".into());
    }

    let permissions = normalize_permissions(read_app_config(&web_directory).permissions);
    if !permissions.contains(&SessioAppPermission::Downloads) {
        return Err("App config does not grant the downloads permission".into());
    }

    let segments = validate_app_relative_path(&request.relative_path)?;
    let relative_path = segments.join("/");
    let file_name = segments
        .last()
        .expect("validated app file path should have a file name");
    if file_name.eq_ignore_ascii_case("config.json") {
        return Err("App file writes cannot replace config.json".into());
    }

    let mut parent = web_directory.clone();
    for segment in &segments[..segments.len() - 1] {
        parent.push(segment);
        match fs::symlink_metadata(&parent) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("App file path cannot pass through a symbolic link".into());
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("App file parent must be a directory".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&parent)
                    .map_err(|error| format!("Could not create app file directory: {error}"))?;
            }
            Err(error) => return Err(format!("Could not inspect app file directory: {error}")),
        }
    }

    let target = parent.join(file_name);
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("App file target must be a regular file".into());
        }
        if !request.overwrite {
            return Err("App file already exists; set overwrite to true to replace it".into());
        }
    }

    let bytes = decode_app_file_data(request.data, request.encoding)?;
    let mut options = OpenOptions::new();
    options.write(true);
    if request.overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options
        .open(&target)
        .map_err(|error| format!("Could not open app file for writing: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("Could not write app file: {error}"))?;

    Ok(SessioAppFileWriteResult {
        relative_path,
        bytes_written: bytes.len(),
    })
}

fn validate_app_relative_path(value: &str) -> Result<Vec<&str>, String> {
    if value.is_empty() || value.len() > 512 || value.starts_with('/') {
        return Err("App file path must be a non-empty relative path of at most 512 bytes".into());
    }
    if value.contains(['\\', '\0']) {
        return Err("App file path cannot contain backslashes or null bytes".into());
    }
    let segments = value.split('/').collect::<Vec<_>>();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || *segment == "."
            || *segment == ".."
            || segment.starts_with('.')
            || segment.ends_with(['.', ' '])
            || segment.contains(':')
            || is_windows_reserved_name(segment)
    }) {
        return Err("App file path contains a disallowed segment".into());
    }
    Ok(segments)
}

fn is_windows_reserved_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

fn decode_app_file_data(data: String, encoding: SessioAppFileEncoding) -> Result<Vec<u8>, String> {
    let bytes = match encoding {
        SessioAppFileEncoding::Utf8 => data.into_bytes(),
        SessioAppFileEncoding::Base64 => {
            let max_encoded_len = MAX_APP_FILE_BYTES.div_ceil(3) * 4;
            if data.len() > max_encoded_len {
                return Err(format!(
                    "App file exceeds the {MAX_APP_FILE_BYTES} byte limit"
                ));
            }
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "App file contains invalid base64 data".to_string())?
        }
    };
    if bytes.len() > MAX_APP_FILE_BYTES {
        return Err(format!(
            "App file exceeds the {MAX_APP_FILE_BYTES} byte limit"
        ));
    }
    Ok(bytes)
}

fn find_logo_path(directory: &Path) -> Option<std::path::PathBuf> {
    const LOGO_NAMES: [&str; 5] = ["logo.svg", "logo.png", "logo.webp", "logo.jpg", "logo.jpeg"];

    let entries = fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
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
        fs::create_dir_all(exact.join("web")).expect("create exact app");
        fs::create_dir_all(fallback.join("web")).expect("create fallback app");
        fs::create_dir_all(ambiguous.join("web")).expect("create ambiguous app");
        fs::write(exact.join("web/sales-report.html"), "<html></html>").expect("write exact html");
        fs::write(exact.join("web/other.html"), "<html></html>").expect("write secondary html");
        fs::write(fallback.join("web/index.HTML"), "<html></html>").expect("write fallback html");
        fs::write(ambiguous.join("web/one.html"), "<html></html>").expect("write first html");
        fs::write(ambiguous.join("web/two.html"), "<html></html>").expect("write second html");
        fs::write(
            exact.join("web/config.json"),
            r#"{"nameZh":"销售报告","nameEn":"Sales Report","permissions":["fullscreen","downloads","modals","popups","clipboardWrite","gamepad","autoplay","pointerLock","unknownPermission","pointerLock"]}"#,
        )
        .expect("write app config");

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
        assert_eq!(apps[2].name_zh.as_deref(), Some("销售报告"));
        assert_eq!(apps[2].name_en.as_deref(), Some("Sales Report"));
        assert_eq!(
            apps[2].permissions,
            vec![
                SessioAppPermission::Fullscreen,
                SessioAppPermission::Downloads,
                SessioAppPermission::Modals,
                SessioAppPermission::Popups,
                SessioAppPermission::ClipboardWrite,
                SessioAppPermission::Gamepad,
                SessioAppPermission::Autoplay,
                SessioAppPermission::PointerLock,
            ]
        );
        assert_eq!(
            serde_json::to_value(&apps[2].permissions).expect("serialize permissions"),
            serde_json::json!([
                "fullscreen",
                "downloads",
                "modals",
                "popups",
                "clipboardWrite",
                "gamepad",
                "autoplay",
                "pointerLock"
            ])
        );
        assert!(apps[1].permissions.is_empty());

        fs::remove_dir_all(&root).expect("remove test apps");
    }

    #[test]
    fn resolves_logo_files_in_priority_order() {
        let root = test_root();
        let app = root.join("brand");
        fs::create_dir_all(app.join("web")).expect("create app");
        fs::write(app.join("web/logo.jpeg"), []).expect("write jpeg logo");
        fs::write(app.join("web/LOGO.PNG"), []).expect("write png logo");

        let info = app_info(&app).expect("read app info");
        assert_eq!(
            info.logo_path
                .as_deref()
                .and_then(|path| Path::new(path).file_name()),
            Some(std::ffi::OsStr::new("LOGO.PNG"))
        );

        fs::remove_dir_all(&root).expect("remove test app");
    }

    #[test]
    fn writes_only_granted_files_inside_the_app_web_directory() {
        let root = test_root();
        let app = root.join("writer");
        let web = app.join("web");
        fs::create_dir_all(&web).expect("create writer app");
        fs::write(web.join("config.json"), r#"{"permissions":["downloads"]}"#)
            .expect("write writer config");

        let text_result = write_app_file_in(
            &root,
            SessioAppFileWriteRequest {
                app_directory_path: app.to_string_lossy().into_owned(),
                relative_path: "exports/state.json".into(),
                data: r#"{"ready":true}"#.into(),
                encoding: SessioAppFileEncoding::Utf8,
                overwrite: false,
            },
        )
        .expect("write app text file");
        assert_eq!(
            text_result,
            SessioAppFileWriteResult {
                relative_path: "exports/state.json".into(),
                bytes_written: 14,
            }
        );
        assert_eq!(
            fs::read_to_string(web.join("exports/state.json")).expect("read app text file"),
            r#"{"ready":true}"#
        );

        let binary_result = write_app_file_in(
            &root,
            SessioAppFileWriteRequest {
                app_directory_path: app.to_string_lossy().into_owned(),
                relative_path: "exports/image.bin".into(),
                data: "AAEC".into(),
                encoding: SessioAppFileEncoding::Base64,
                overwrite: false,
            },
        )
        .expect("write app binary file");
        assert_eq!(binary_result.bytes_written, 3);
        assert_eq!(
            fs::read(web.join("exports/image.bin")).expect("read app binary file"),
            vec![0, 1, 2]
        );

        for relative_path in [
            "../outside.txt",
            ".hidden",
            "config.json",
            "config.json.",
            "exports/NUL.txt",
        ] {
            let error = write_app_file_in(
                &root,
                SessioAppFileWriteRequest {
                    app_directory_path: app.to_string_lossy().into_owned(),
                    relative_path: relative_path.into(),
                    data: "blocked".into(),
                    encoding: SessioAppFileEncoding::Utf8,
                    overwrite: true,
                },
            )
            .expect_err("reject protected app path");
            assert!(!error.is_empty());
        }

        let overwrite_error = write_app_file_in(
            &root,
            SessioAppFileWriteRequest {
                app_directory_path: app.to_string_lossy().into_owned(),
                relative_path: "exports/state.json".into(),
                data: "replacement".into(),
                encoding: SessioAppFileEncoding::Utf8,
                overwrite: false,
            },
        )
        .expect_err("require explicit overwrite");
        assert!(overwrite_error.contains("overwrite"));

        fs::write(web.join("config.json"), r#"{"permissions":[]}"#)
            .expect("remove writer permission");
        let permission_error = write_app_file_in(
            &root,
            SessioAppFileWriteRequest {
                app_directory_path: app.to_string_lossy().into_owned(),
                relative_path: "denied.txt".into(),
                data: "blocked".into(),
                encoding: SessioAppFileEncoding::Utf8,
                overwrite: false,
            },
        )
        .expect_err("require downloads permission");
        assert!(permission_error.contains("downloads"));

        fs::remove_dir_all(&root).expect("remove writer app");
    }

    #[test]
    fn missing_root_is_an_empty_catalog() {
        let root = test_root();
        assert!(list_apps_in(&root).expect("list missing root").is_empty());
    }
}
