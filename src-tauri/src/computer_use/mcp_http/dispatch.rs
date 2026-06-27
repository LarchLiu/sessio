//! Dispatch a validated MCP request into the [`ComputerUseHost`].
//!
//! Pure routing: given an authenticated session id, the live desktop-control
//! permission status, and a parsed [`McpRequest`], produce an [`McpResponse`].
//! All gating (settings / permission / approval / lease / snapshot) is enforced
//! by the host, so this layer only translates JSON args ↔ host calls and maps
//! host errors into MCP tool-error content.

use serde_json::{json, Value};

use crate::computer_use::host::ComputerUseHost;
use crate::computer_use::lease::SnapshotId;
use crate::computer_use::provider::{AppTarget, ScrollDirection};
use crate::desktop_control::DesktopControlPermissionStatus;

use super::protocol::{
    initialize_result, tool_error_result, tool_text_result, tools_list_result, McpRequest,
    McpResponse,
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
            Ok(tool_text_result(
                serde_json::to_string(&status).unwrap_or_default(),
            ))
        }
        "computer_list_apps" => {
            let apps = host.list_apps(perm).map_err(|e| e.to_string())?;
            Ok(tool_text_result(
                serde_json::to_string(&apps).unwrap_or_default(),
            ))
        }
        "computer_start" => {
            let target = parse_target(args)?;
            let lease = host.start(session_id, target, perm).map_err(|e| e.to_string())?;
            Ok(tool_text_result(json!({ "lease": lease }).to_string()))
        }
        "computer_get_app_state" => {
            let state = host.get_app_state(session_id, perm).map_err(|e| e.to_string())?;
            Ok(tool_text_result(
                serde_json::to_string(&state).unwrap_or_default(),
            ))
        }
        "computer_click_element" => {
            let snapshot = parse_snapshot(args)?;
            let element = arg_str(args, "elementId")?;
            host.click_element(session_id, &snapshot, &element, perm)
                .map_err(|e| e.to_string())?;
            Ok(tool_text_result("ok"))
        }
        "computer_type_text" => {
            let snapshot = parse_snapshot(args)?;
            let text = arg_str(args, "text")?;
            host.type_text(session_id, &snapshot, &text, perm)
                .map_err(|e| e.to_string())?;
            Ok(tool_text_result("ok"))
        }
        "computer_press_key" => {
            let snapshot = parse_snapshot(args)?;
            let key = arg_str(args, "key")?;
            host.press_key(session_id, &snapshot, &key, perm)
                .map_err(|e| e.to_string())?;
            Ok(tool_text_result("ok"))
        }
        "computer_scroll" => {
            let snapshot = parse_snapshot(args)?;
            let direction = parse_direction(args)?;
            let amount = args
                .get("amount")
                .and_then(|a| a.as_i64())
                .unwrap_or(0) as i32;
            host.scroll(session_id, &snapshot, direction, amount, perm)
                .map_err(|e| e.to_string())?;
            Ok(tool_text_result("ok"))
        }
        "computer_stop" => {
            host.stop(session_id);
            Ok(tool_text_result("ok"))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn parse_target(args: &Value) -> Result<AppTarget, String> {
    Ok(AppTarget {
        app_id: arg_str(args, "appId")?,
        window_id: args
            .get("windowId")
            .and_then(|w| w.as_str())
            .map(|s| s.to_string()),
    })
}

fn parse_snapshot(args: &Value) -> Result<SnapshotId, String> {
    Ok(SnapshotId(arg_str(args, "snapshotId")?))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::host::ComputerUseHost;
    use crate::computer_use::provider::FakeProvider;
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
            ComputerUseSettings {
                enabled: true,
                allow_input_injection: true,
                allow_foreground_takeover: true,
            },
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
        h.approvals().approve_app("s1", &"com.example.app".to_string());

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
            &call("tools/call", json!({ "name": "computer_get_app_state", "arguments": {} })),
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
                assert!(result.get("isError").is_none(), "click should succeed: {result}")
            }
            _ => panic!("expected result"),
        }
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
        h.approvals().approve_app("s1", &"com.example.app".to_string());
        dispatch(&h, "s1", &p, &call("tools/call", json!({ "name": "computer_start", "arguments": { "appId": "com.example.app" } })));
        let first = dispatch(&h, "s1", &p, &call("tools/call", json!({ "name": "computer_get_app_state", "arguments": {} })));
        let stale_id = match first {
            McpResponse::Result { result, .. } => {
                let text = result["content"][0]["text"].as_str().unwrap();
                serde_json::from_str::<Value>(text).unwrap()["snapshotId"].as_str().unwrap().to_string()
            }
            _ => panic!(),
        };
        // capture again to invalidate
        dispatch(&h, "s1", &p, &call("tools/call", json!({ "name": "computer_get_app_state", "arguments": {} })));
        let resp = dispatch(
            &h,
            "s1",
            &p,
            &call("tools/call", json!({ "name": "computer_type_text", "arguments": { "snapshotId": stale_id, "text": "hi" } })),
        );
        match resp {
            McpResponse::Result { result, .. } => assert_eq!(result["isError"], true),
            _ => panic!("expected isError result"),
        }
    }
}
