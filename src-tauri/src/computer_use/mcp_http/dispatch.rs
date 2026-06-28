//! Dispatch a validated MCP request into the [`ComputerUseHost`].
//!
//! Pure routing: given an authenticated session id, the live desktop-control
//! permission status, and a parsed [`McpRequest`], produce an [`McpResponse`].
//! All gating (settings / permission / approval / lease / snapshot) is enforced
//! by the host, so this layer only translates JSON args ↔ host calls and maps
//! host errors into MCP tool-error content.

use serde_json::{json, Value};

use crate::computer_use::host::{ComputerUseError, ComputerUseHost};
use crate::computer_use::lease::SnapshotId;
use crate::computer_use::onboarding::{self, PermissionKind};
use crate::computer_use::permissions::PermissionDenied;
use crate::computer_use::provider::{AppTarget, CoordinateSpace, Point, ScrollDirection};
use crate::desktop_control::DesktopControlPermissionStatus;

use super::protocol::{
    initialize_result, tool_app_state_result, tool_error_result, tool_json_result,
    tool_text_result, tools_list_result, McpRequest, McpResponse,
};

/// Route an MCP request for `session_id` to the host.
pub fn dispatch(
    host: &ComputerUseHost,
    session_id: &str,
    perm: &DesktopControlPermissionStatus,
    request: &McpRequest,
) -> McpResponse {
    match request.method.as_str() {
        "initialize" => McpResponse::result(request.id.clone(), initialize_result()),
        "notifications/initialized" | "initialized" => {
            // Notifications carry no id and expect no result; reply empty.
            McpResponse::result(request.id.clone(), json!({}))
        }
        "tools/list" => McpResponse::result(request.id.clone(), tools_list_result()),
        "tools/call" => dispatch_tool_call(host, session_id, perm, request),
        other => McpResponse::error(
            request.id.clone(),
            -32601,
            format!("method not found: {other}"),
        ),
    }
}

fn dispatch_tool_call(
    host: &ComputerUseHost,
    session_id: &str,
    perm: &DesktopControlPermissionStatus,
    request: &McpRequest,
) -> McpResponse {
    let name = request.params.get("name").and_then(|n| n.as_str());
    let args = request
        .params
        .get("arguments")
        .cloned()
        .unwrap_or(json!({}));
    let id = request.id.clone();

    let Some(name) = name else {
        return McpResponse::error(id, -32602, "missing tool name");
    };

    let result = run_tool(host, session_id, perm, name, &args);
    match result {
        Ok(value) => McpResponse::result(id, value),
        // Tool-level failures are returned as a successful JSON-RPC response with
        // isError content, per MCP, so the model can read and react to them.
        Err(message) => McpResponse::result(id, tool_error_result(message)),
    }
}

fn run_tool(
    host: &ComputerUseHost,
    session_id: &str,
    perm: &DesktopControlPermissionStatus,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "computer_status" => {
            let status = host.status(session_id, perm);
            Ok(tool_json_result(
                serde_json::to_value(&status).unwrap_or_default(),
            ))
        }
        "computer_permissions" => Ok(tool_json_result(
            serde_json::to_value(onboarding::permissions_status(perm)).unwrap_or_default(),
        )),
        "computer_grant" => {
            let permission = PermissionKind::parse(&arg_str(args, "permission")?)?;
            let result = onboarding::grant_permission(permission, perm)?;
            Ok(tool_json_result(
                serde_json::to_value(&result).unwrap_or_default(),
            ))
        }
        "computer_list_apps" => {
            let apps = host.list_apps(perm).map_err(host_error_message)?;
            Ok(tool_json_result(
                serde_json::to_value(&apps).unwrap_or_default(),
            ))
        }
        "computer_start" => {
            let target = parse_target(args)?;
            let lease = host
                .start(session_id, target, perm)
                .map_err(host_error_message)?;
            Ok(tool_json_result(json!({ "lease": lease })))
        }
        "computer_launch_app" => {
            let target = parse_target(args)?;
            let result = host
                .launch_app(session_id, target, perm)
                .map_err(host_error_message)?;
            Ok(tool_json_result(
                serde_json::to_value(&result).unwrap_or_default(),
            ))
        }
        "computer_raise_app" => {
            let target = parse_target(args)?;
            let result = host
                .raise_app(session_id, target, perm)
                .map_err(host_error_message)?;
            Ok(tool_json_result(
                serde_json::to_value(&result).unwrap_or_default(),
            ))
        }
        "computer_get_app_state" => {
            let state = match parse_optional_target(args)? {
                Some(target) => host.get_app_state_for_target(session_id, target, perm),
                None => host.get_app_state(session_id, perm),
            }
            .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_click" => {
            let snapshot = parse_snapshot(args)?;
            let state = if let Some(element) = optional_element_ref(args) {
                host.click_element(session_id, &snapshot, &element, perm)
            } else {
                let point = parse_point(args, "x", "y")?;
                let coord_space = parse_coordinate_space(args)?;
                host.click_at(session_id, &snapshot, point, coord_space, perm)
            }
            .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_click_element" => {
            let snapshot = parse_snapshot(args)?;
            let element = arg_element_ref(args)?;
            let state = host
                .click_element(session_id, &snapshot, &element, perm)
                .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_click_at" => {
            let snapshot = parse_snapshot(args)?;
            let point = parse_point(args, "x", "y")?;
            let coord_space = parse_coordinate_space(args)?;
            let state = host
                .click_at(session_id, &snapshot, point, coord_space, perm)
                .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_secondary_click" | "computer_perform_secondary_action" => {
            let snapshot = parse_snapshot(args)?;
            let state = if let Some(element) = optional_element_ref(args) {
                host.secondary_click_element(session_id, &snapshot, &element, perm)
            } else {
                let point = parse_point(args, "x", "y")?;
                let coord_space = parse_coordinate_space(args)?;
                host.secondary_click(session_id, &snapshot, point, coord_space, perm)
            }
            .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_double_click" => {
            let snapshot = parse_snapshot(args)?;
            let point = parse_point(args, "x", "y")?;
            let coord_space = parse_coordinate_space(args)?;
            let state = host
                .double_click(session_id, &snapshot, point, coord_space, perm)
                .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_drag" => {
            let snapshot = parse_snapshot(args)?;
            let from = parse_point(args, "fromX", "fromY")?;
            let to = parse_point(args, "toX", "toY")?;
            let coord_space = parse_coordinate_space(args)?;
            let state = host
                .drag(session_id, &snapshot, from, to, coord_space, perm)
                .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_set_value" => {
            let snapshot = parse_snapshot(args)?;
            let element = arg_element_ref(args)?;
            let value = arg_str(args, "value")?;
            let state = host
                .set_value(session_id, &snapshot, &element, &value, perm)
                .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_type_text" => {
            let snapshot = parse_snapshot(args)?;
            let text = arg_str(args, "text")?;
            let state = host
                .type_text(session_id, &snapshot, &text, perm)
                .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_press_key" => {
            let snapshot = parse_snapshot(args)?;
            let key = arg_str(args, "key")?;
            let state = host
                .press_key(session_id, &snapshot, &key, perm)
                .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_scroll" => {
            let snapshot = parse_snapshot(args)?;
            let direction = parse_direction(args)?;
            let amount = args.get("amount").and_then(|a| a.as_i64()).unwrap_or(0) as i32;
            let state = if let Some(element) = optional_element_ref(args) {
                host.scroll_element(session_id, &snapshot, &element, direction, amount, perm)
            } else {
                host.scroll(session_id, &snapshot, direction, amount, perm)
            }
            .map_err(host_error_message)?;
            Ok(tool_app_state_result(
                serde_json::to_value(&state).unwrap_or_default(),
            ))
        }
        "computer_stop" => {
            host.stop(session_id);
            Ok(tool_text_result("ok"))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn host_error_message(error: ComputerUseError) -> String {
    match error {
        ComputerUseError::Permission(PermissionDenied::Observe) => {
            "permission_missing:screenshots: screen capture permission is required. Call computer_permissions for status, then computer_grant with permission=\"screenshots\"."
                .into()
        }
        ComputerUseError::Permission(PermissionDenied::Inspect) => {
            "permission_missing:accessibility: accessibility permission is required. Call computer_permissions for status, then computer_grant with permission=\"accessibility\"."
                .into()
        }
        ComputerUseError::Permission(PermissionDenied::Control) => {
            "permission_missing:control: input control is unavailable. Call computer_permissions; on macOS this usually requires Accessibility."
                .into()
        }
        other => other.to_string(),
    }
}

fn parse_target(args: &Value) -> Result<AppTarget, String> {
    Ok(AppTarget {
        app_id: arg_str_any(args, &["appId", "bundle"])?,
        window_id: args
            .get("windowId")
            .and_then(|w| w.as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_optional_target(args: &Value) -> Result<Option<AppTarget>, String> {
    if args.get("appId").is_none() && args.get("bundle").is_none() {
        return Ok(None);
    }
    parse_target(args).map(Some)
}

fn parse_snapshot(args: &Value) -> Result<SnapshotId, String> {
    Ok(SnapshotId(arg_str(args, "snapshotId")?))
}

fn parse_point(args: &Value, x_key: &str, y_key: &str) -> Result<Point, String> {
    Ok(Point {
        x: arg_f32(args, x_key)?,
        y: arg_f32(args, y_key)?,
    })
}

fn parse_coordinate_space(args: &Value) -> Result<CoordinateSpace, String> {
    match args
        .get("coordSpace")
        .or_else(|| args.get("coordinateSpace"))
        .or_else(|| args.get("coord_space"))
        .and_then(|v| v.as_str())
        .unwrap_or("screenshot")
    {
        "screenshot" => Ok(CoordinateSpace::Screenshot),
        "screen" => Ok(CoordinateSpace::Screen),
        other => Err(format!("invalid coordSpace: {other}")),
    }
}

fn parse_direction(args: &Value) -> Result<ScrollDirection, String> {
    match args.get("direction").and_then(|d| d.as_str()) {
        Some("up") => Ok(ScrollDirection::Up),
        Some("down") => Ok(ScrollDirection::Down),
        Some("left") => Ok(ScrollDirection::Left),
        Some("right") => Ok(ScrollDirection::Right),
        Some(other) => Err(format!("invalid direction: {other}")),
        None => Err("missing direction".into()),
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing argument: {key}"))
}

fn arg_str_any(args: &Value, keys: &[&str]) -> Result<String, String> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing argument: {}", keys.join("|")))
}

fn optional_element_ref(args: &Value) -> Option<String> {
    args.get("elementId")
        .or_else(|| args.get("ref"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn arg_element_ref(args: &Value) -> Result<String, String> {
    optional_element_ref(args).ok_or_else(|| "missing argument: elementId|ref".into())
}

fn arg_f32(args: &Value, key: &str) -> Result<f32, String> {
    args.get(key)
        .and_then(|v| v.as_f64())
        .map(|n| n as f32)
        .ok_or_else(|| format!("missing argument: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::host::ComputerUseHost;
    use crate::computer_use::provider::{FakeProvider, InstalledApp};
    use crate::computer_use::settings::ComputerUseSettings;
    use crate::desktop_control::{
        DesktopControlInputs, DesktopControlPermissionStatus, DesktopPlatform, PermissionTier,
    };
    use std::sync::Arc;

    fn perm() -> DesktopControlPermissionStatus {
        DesktopControlPermissionStatus::derive(DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(true, true),
            accessibility: PermissionTier::new(true, true),
            input_injection_supported: true,
        })
    }

    fn host() -> ComputerUseHost {
        ComputerUseHost::new(
            Arc::new(FakeProvider::default()),
            ComputerUseSettings::enabled(),
        )
    }

    fn host_with_stopped_app() -> (ComputerUseHost, Arc<FakeProvider>) {
        let provider = Arc::new(FakeProvider::with_apps(vec![InstalledApp {
            id: "com.example.installed".into(),
            name: "Installed".into(),
            pid: None,
            running: false,
        }]));
        (
            ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled()),
            provider,
        )
    }

    fn call(method: &str, params: Value) -> McpRequest {
        McpRequest {
            id: json!(1),
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn initialize_and_tools_list_are_handled() {
        let h = host();
        let p = perm();
        let init = dispatch(&h, "s1", &p, &call("initialize", json!({})));
        match init {
            McpResponse::Result { result, .. } => {
                assert_eq!(result["serverInfo"]["name"], "sessio-computer-use")
            }
            _ => panic!("expected result"),
        }
        let list = dispatch(&h, "s1", &p, &call("tools/list", json!({})));
        match list {
            McpResponse::Result { result, .. } => {
                assert!(result["tools"].as_array().unwrap().len() >= 9)
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn permissions_tool_returns_onboarding_status() {
        let h = host();
        let p = DesktopControlPermissionStatus::derive(DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(false, true),
            accessibility: PermissionTier::new(true, true),
            input_injection_supported: true,
        });

        let resp = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_permissions", "arguments": {} }),
            ),
        );

        match resp {
            McpResponse::Result { result, .. } => {
                assert_eq!(result["structuredContent"]["ready"], false);
                assert_eq!(result["structuredContent"]["missing"][0], "screenshots");
                assert_eq!(
                    result["structuredContent"]["requirements"][0]["code"],
                    "missing_screenshots"
                );
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn unknown_method_is_jsonrpc_error() {
        let h = host();
        let resp = dispatch(&h, "s1", &perm(), &call("nope", json!({})));
        match resp {
            McpResponse::Error { code, .. } => assert_eq!(code, -32601),
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn full_flow_start_capture_click() {
        let h = host();
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.app".to_string());

        // start
        let start = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_start", "arguments": { "appId": "com.example.app" } }),
            ),
        );
        assert!(matches!(start, McpResponse::Result { .. }));

        // get_app_state → snapshot id
        let state = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_get_app_state", "arguments": {} }),
            ),
        );
        let snapshot_id = match state {
            McpResponse::Result { result, .. } => {
                let text = result["content"][0]["text"].as_str().unwrap();
                let parsed: Value = serde_json::from_str(text).unwrap();
                parsed["snapshotId"].as_str().unwrap().to_string()
            }
            _ => panic!("expected result"),
        };

        // click against the fresh snapshot
        let click = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_click_element", "arguments": { "snapshotId": snapshot_id, "elementId": "el-1" } }),
            ),
        );
        match click {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "click should succeed: {result}"
                );
                let text = result["content"][0]["text"].as_str().unwrap();
                let parsed: Value = serde_json::from_str(text).unwrap();
                assert_ne!(parsed["snapshotId"].as_str().unwrap(), snapshot_id);
                assert_eq!(
                    result["structuredContent"]["snapshotId"],
                    parsed["snapshotId"]
                );
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn unified_click_prefers_ax_element_id() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.app".to_string());
        h.start(
            "s1",
            AppTarget {
                app_id: "com.example.app".into(),
                window_id: None,
            },
            &p,
        )
        .unwrap();
        let state = h.get_app_state("s1", &p).unwrap();

        let click = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({
                    "name": "computer_click",
                    "arguments": {
                        "snapshotId": state.snapshot_id,
                        "elementId": "el-1",
                        "x": 360,
                        "y": 225
                    }
                }),
            ),
        );

        match click {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "click should succeed: {result}"
                );
                assert_eq!(
                    provider.actions(),
                    vec!["click:com.example.app:el-1".to_string()]
                );
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn unified_click_accepts_ref_alias() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.app".to_string());
        h.start(
            "s1",
            AppTarget {
                app_id: "com.example.app".into(),
                window_id: None,
            },
            &p,
        )
        .unwrap();
        let state = h.get_app_state("s1", &p).unwrap();

        let click = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({
                    "name": "computer_click",
                    "arguments": {
                        "snapshotId": state.snapshot_id,
                        "ref": "el-1"
                    }
                }),
            ),
        );

        match click {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "click should succeed: {result}"
                );
                assert_eq!(
                    provider.actions(),
                    vec!["click:com.example.app:el-1".to_string()]
                );
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn perform_secondary_action_prefers_ax_ref() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.app".to_string());
        h.start(
            "s1",
            AppTarget {
                app_id: "com.example.app".into(),
                window_id: None,
            },
            &p,
        )
        .unwrap();
        let state = h.get_app_state("s1", &p).unwrap();

        let click = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({
                    "name": "computer_perform_secondary_action",
                    "arguments": {
                        "snapshotId": state.snapshot_id,
                        "ref": "el-1",
                        "x": 360,
                        "y": 225
                    }
                }),
            ),
        );

        match click {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "secondary action should succeed: {result}"
                );
                assert_eq!(
                    provider.actions(),
                    vec!["secondary_click_element:com.example.app:el-1".to_string()]
                );
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn coordinate_tool_defaults_to_screenshot_space() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.app".to_string());
        h.start(
            "s1",
            AppTarget {
                app_id: "com.example.app".into(),
                window_id: None,
            },
            &p,
        )
        .unwrap();
        let state = h.get_app_state("s1", &p).unwrap();

        let click = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_click_at", "arguments": { "snapshotId": state.snapshot_id, "x": 360, "y": 225 } }),
            ),
        );

        match click {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "click_at should succeed: {result}"
                );
                assert_eq!(
                    provider.actions(),
                    vec!["click_at:com.example.app:190.0,132.5".to_string()]
                );
                assert!(result["structuredContent"]["snapshotId"].is_string());
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn coordinate_tool_accepts_coord_space_alias() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.app".to_string());
        h.start(
            "s1",
            AppTarget {
                app_id: "com.example.app".into(),
                window_id: None,
            },
            &p,
        )
        .unwrap();
        let state = h.get_app_state("s1", &p).unwrap();

        let click = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_click_at", "arguments": { "snapshotId": state.snapshot_id, "x": 360, "y": 225, "coord_space": "screen" } }),
            ),
        );

        match click {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "click_at should succeed: {result}"
                );
                assert_eq!(
                    provider.actions(),
                    vec!["click_at:com.example.app:360.0,225.0".to_string()]
                );
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn get_app_state_with_target_launches_after_approval() {
        let (h, provider) = host_with_stopped_app();
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.installed".to_string());

        let state = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_get_app_state", "arguments": { "appId": "com.example.installed" } }),
            ),
        );
        match state {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "state should succeed: {result}"
                );
                let text = result["content"][0]["text"].as_str().unwrap();
                let parsed: Value = serde_json::from_str(text).unwrap();
                assert_eq!(parsed["launched"], true);
                assert_eq!(parsed["target"]["appId"], "com.example.installed");
            }
            _ => panic!("expected result"),
        }
        assert_eq!(
            provider.actions(),
            vec!["launch:com.example.installed".to_string()]
        );
    }

    #[test]
    fn get_app_state_accepts_bundle_alias() {
        let (h, _provider) = host_with_stopped_app();
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.installed".to_string());

        let state = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_get_app_state", "arguments": { "bundle": "com.example.installed" } }),
            ),
        );
        match state {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "state should succeed: {result}"
                );
                assert_eq!(
                    result["structuredContent"]["target"]["appId"],
                    "com.example.installed"
                );
            }
            _ => panic!("expected result"),
        }
    }

    #[test]
    fn launch_app_without_app_approval_is_tool_error() {
        let (h, provider) = host_with_stopped_app();
        let p = perm();
        h.approvals().approve_session("s1");

        let resp = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_launch_app", "arguments": { "appId": "com.example.installed" } }),
            ),
        );
        match resp {
            McpResponse::Result { result, .. } => assert_eq!(result["isError"], true),
            _ => panic!("expected isError result"),
        }
        assert!(provider.actions().is_empty());
    }

    #[test]
    fn raise_app_restores_target_after_approval() {
        let (h, provider) = host_with_stopped_app();
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.installed".to_string());

        let resp = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_raise_app", "arguments": { "bundle": "com.example.installed" } }),
            ),
        );
        match resp {
            McpResponse::Result { result, .. } => {
                assert!(
                    result.get("isError").is_none(),
                    "raise should succeed: {result}"
                );
                assert_eq!(
                    result["structuredContent"]["target"]["appId"],
                    "com.example.installed"
                );
                assert_eq!(result["structuredContent"]["launched"], true);
                assert_eq!(result["structuredContent"]["activated"], true);
                assert_eq!(result["structuredContent"]["visible"], true);
            }
            _ => panic!("expected result"),
        }
        assert_eq!(
            provider.actions(),
            vec!["raise:com.example.installed".to_string()]
        );
    }

    #[test]
    fn tool_failure_returns_iserror_content_not_jsonrpc_error() {
        let h = host();
        let p = perm();
        // start without approval → host returns an Approval error, surfaced as
        // an isError tool result (still a JSON-RPC success envelope).
        let resp = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_start", "arguments": { "appId": "com.example.app" } }),
            ),
        );
        match resp {
            McpResponse::Result { result, .. } => {
                assert_eq!(result["isError"], true);
            }
            _ => panic!("tool failure must be a result with isError, not a JSON-RPC error"),
        }
    }

    #[test]
    fn stale_snapshot_action_is_reported_as_tool_error() {
        let h = host();
        let p = perm();
        h.approvals().approve_session("s1");
        h.approvals()
            .approve_app("s1", &"com.example.app".to_string());
        dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_start", "arguments": { "appId": "com.example.app" } }),
            ),
        );
        let first = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_get_app_state", "arguments": {} }),
            ),
        );
        let stale_id = match first {
            McpResponse::Result { result, .. } => {
                let text = result["content"][0]["text"].as_str().unwrap();
                serde_json::from_str::<Value>(text).unwrap()["snapshotId"]
                    .as_str()
                    .unwrap()
                    .to_string()
            }
            _ => panic!(),
        };
        // capture again to invalidate
        dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_get_app_state", "arguments": {} }),
            ),
        );
        let resp = dispatch(
            &h,
            "s1",
            &p,
            &call(
                "tools/call",
                json!({ "name": "computer_type_text", "arguments": { "snapshotId": stale_id, "text": "hi" } }),
            ),
        );
        match resp {
            McpResponse::Result { result, .. } => assert_eq!(result["isError"], true),
            _ => panic!("expected isError result"),
        }
    }
}
