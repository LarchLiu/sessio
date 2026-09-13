use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use uuid::Uuid;

use crate::app_paths;

const MAX_APP_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_APP_HTML_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
struct AppResourceGrant {
    web_root: PathBuf,
    webview_label: String,
}

#[derive(Default)]
pub(crate) struct AppResourceGrants {
    grants: Mutex<HashMap<String, AppResourceGrant>>,
}

#[tauri::command]
pub(crate) fn create_sessio_app_resource_grant(
    app_directory_path: String,
    window: WebviewWindow,
    grants: State<'_, AppResourceGrants>,
) -> Result<String, String> {
    let web_root = validate_app_web_root(Path::new(&app_directory_path))?;
    let token = Uuid::new_v4().simple().to_string();
    let grant = AppResourceGrant {
        web_root,
        webview_label: window.label().to_string(),
    };
    grants
        .grants
        .lock()
        .map_err(|_| "App resource grant registry is unavailable".to_string())?
        .insert(token.clone(), grant);
    Ok(token)
}

#[tauri::command]
pub(crate) fn revoke_sessio_app_resource_grant(
    token: String,
    grants: State<'_, AppResourceGrants>,
) -> Result<(), String> {
    if token.len() > 128 || token.is_empty() {
        return Err("Invalid app resource grant".to_string());
    }
    grants
        .grants
        .lock()
        .map_err(|_| "App resource grant registry is unavailable".to_string())?
        .remove(&token);
    Ok(())
}

#[tauri::command]
pub(crate) fn read_sessio_app_html(
    app_directory_path: String,
    html_path: String,
) -> Result<String, String> {
    let web_root = validate_app_web_root(Path::new(&app_directory_path))?;
    let html_path =
        fs::canonicalize(&html_path).map_err(|error| format!("Invalid App HTML path: {error}"))?;
    if !html_path.starts_with(&web_root)
        || !html_path.is_file()
        || !html_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
    {
        return Err("App HTML path must be an HTML file inside the App web directory".into());
    }
    let metadata = fs::metadata(&html_path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_APP_HTML_BYTES {
        return Err("App HTML file exceeds the 64 MiB limit".into());
    }
    fs::read_to_string(&html_path).map_err(|error| format!("Could not read App HTML: {error}"))
}

pub(crate) fn serve(
    app: &AppHandle,
    webview_label: &str,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let method = request.method().as_str();
    if method == "OPTIONS" {
        return response(StatusCode::NO_CONTENT, "text/plain", Vec::new());
    }
    if method != "GET" && method != "HEAD" {
        return response(StatusCode::METHOD_NOT_ALLOWED, "text/plain", Vec::new());
    }

    let path = request.uri().path();
    let Some((token, relative_path)) = parse_resource_path(path) else {
        return response(
            StatusCode::BAD_REQUEST,
            "text/plain",
            b"invalid resource path".to_vec(),
        );
    };

    let grant = {
        let Some(state) = app.try_state::<AppResourceGrants>() else {
            return response(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", Vec::new());
        };
        let Ok(grants) = state.grants.lock() else {
            return response(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", Vec::new());
        };
        let Some(grant) = grants.get(token) else {
            return response(StatusCode::NOT_FOUND, "text/plain", Vec::new());
        };
        if grant.webview_label != webview_label {
            return response(StatusCode::FORBIDDEN, "text/plain", Vec::new());
        }
        grant.clone()
    };

    let Some(path) = resolve_resource_path(&grant.web_root, &relative_path) else {
        return response(StatusCode::FORBIDDEN, "text/plain", Vec::new());
    };
    let Some(content_type) = resource_content_type(&path) else {
        return response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "text/plain", Vec::new());
    };
    let Ok(metadata) = fs::metadata(&path) else {
        return response(StatusCode::NOT_FOUND, "text/plain", Vec::new());
    };
    if !metadata.is_file() {
        return response(StatusCode::NOT_FOUND, "text/plain", Vec::new());
    }
    if metadata.len() > MAX_APP_RESOURCE_BYTES {
        return response(StatusCode::PAYLOAD_TOO_LARGE, "text/plain", Vec::new());
    }
    let body = if method == "HEAD" {
        Vec::new()
    } else {
        match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => return response(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", Vec::new()),
        }
    };
    response(StatusCode::OK, content_type, body)
}

fn response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS")
        .header("Access-Control-Allow-Headers", "Content-Type")
        .header("Cache-Control", "no-store")
        .body(body)
        .expect("app resource response headers are valid")
}

fn parse_resource_path(path: &str) -> Option<(&str, String)> {
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let token = segments.next()?;
    let relative_path = segments.collect::<Vec<_>>().join("/");
    (!relative_path.is_empty()).then_some((token, relative_path))
}

fn validate_app_web_root(app_directory: &Path) -> Result<PathBuf, String> {
    let apps_root = fs::canonicalize(app_paths::apps_dir().map_err(|error| error.to_string())?)
        .map_err(|error| format!("Invalid apps root: {error}"))?;
    let app_directory = fs::canonicalize(app_directory)
        .map_err(|error| format!("Invalid app directory: {error}"))?;
    if app_directory.parent() != Some(apps_root.as_path()) {
        return Err("App directory must be installed directly under the Sessio apps root".into());
    }
    let web_root = fs::canonicalize(app_directory.join("web"))
        .map_err(|error| format!("Invalid app web directory: {error}"))?;
    if !web_root.is_dir() || web_root.parent() != Some(app_directory.as_path()) {
        return Err("App web directory must be a real directory inside the app".into());
    }
    Ok(web_root)
}

fn resolve_resource_path(web_root: &Path, relative_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(relative_path)?;
    if decoded.is_empty() || decoded.contains('\0') || decoded.contains('\\') {
        return None;
    }
    let relative = Path::new(&decoded);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    let path = web_root.join(relative);
    let canonical = fs::canonicalize(path).ok()?;
    canonical.starts_with(web_root).then_some(canonical)
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn resource_content_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "html" | "htm" => Some("text/html; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "js" | "mjs" => Some("application/javascript; charset=utf-8"),
        "json" => Some("application/json; charset=utf-8"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        "glb" => Some("model/gltf-binary"),
        "gltf" => Some("model/gltf+json"),
        "bin" => Some("application/octet-stream"),
        "wasm" => Some("application/wasm"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        "ttf" => Some("font/ttf"),
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "ogg" => Some("audio/ogg"),
        "m4a" => Some("audio/mp4"),
        "aac" => Some("audio/aac"),
        "webm" => Some("audio/webm"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_utf8_and_rejects_malformed_values() {
        assert_eq!(
            percent_decode("assets/%E4%B9%A6.glb").as_deref(),
            Some("assets/书.glb")
        );
        assert!(percent_decode("assets/%ZZ.glb").is_none());
        assert!(percent_decode("assets/%E4.glb").is_none());
    }

    #[test]
    fn resource_content_type_covers_three_assets() {
        assert_eq!(
            resource_content_type(Path::new("scene.glb")),
            Some("model/gltf-binary")
        );
        assert_eq!(
            resource_content_type(Path::new("style.css")),
            Some("text/css; charset=utf-8")
        );
        assert_eq!(
            resource_content_type(Path::new("narration.m4a")),
            Some("audio/mp4")
        );
        assert_eq!(
            resource_content_type(Path::new("narration.aac")),
            Some("audio/aac")
        );
        assert_eq!(resource_content_type(Path::new("secret.exe")), None);
    }
}
