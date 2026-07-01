//! MCP JSON-RPC protocol framing for the computer-use tool server.
//!
//! Implements the subset of the Model Context Protocol the ACP adapters use to
//! discover and invoke tools over HTTP: `initialize`, `tools/list`, and
//! `tools/call`. Parsing/serialization is pure so it is unit-testable without a
//! socket; dispatch of `tools/call` into the host lives in [`super::dispatch`].

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::computer_use::provider::Point;
use crate::computer_use::screenshot_overlay::render_reference_overlay_png;

/// A parsed MCP JSON-RPC request.
#[derive(Debug, Clone, PartialEq)]
pub struct McpRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

/// An MCP JSON-RPC response (success or error).
#[derive(Debug, Clone, PartialEq)]
pub enum McpResponse {
    Result {
        id: Value,
        result: Value,
    },
    Error {
        id: Value,
        code: i64,
        message: String,
    },
}

impl McpRequest {
    /// Parse a JSON-RPC request body. Returns `Err` with a JSON-RPC parse-error
    /// response when the body is not a valid request.
    pub fn parse(body: &str) -> Result<Self, McpResponse> {
        let value: Value = serde_json::from_str(body).map_err(|_| McpResponse::Error {
            id: Value::Null,
            code: -32700,
            message: "parse error".into(),
        })?;
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let method = value
            .get("method")
            .and_then(|m| m.as_str())
            .ok_or_else(|| McpResponse::Error {
                id: id.clone(),
                code: -32600,
                message: "invalid request: missing method".into(),
            })?
            .to_string();
        let params = value.get("params").cloned().unwrap_or(json!({}));
        Ok(Self { id, method, params })
    }
}

impl McpResponse {
    pub fn to_json(&self) -> Value {
        match self {
            McpResponse::Result { id, result } => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            }),
            McpResponse::Error { id, code, message } => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": message },
            }),
        }
    }

    pub fn to_string(&self) -> String {
        self.to_json().to_string()
    }

    pub fn result(id: Value, result: Value) -> Self {
        McpResponse::Result { id, result }
    }

    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        McpResponse::Error {
            id,
            code,
            message: message.into(),
        }
    }
}

/// A single tool definition advertised via `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the tool's input.
    pub input_schema: Value,
}

/// Build the `computer_*` tool list. The set offered to a given session is
/// filtered by the host based on permission tiers; this is the full catalog.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    let no_args = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    let app_arg = json!({
        "type": "object",
        "properties": {
            "appId": { "type": "string" },
            "bundle": { "type": "string" },
            "windowId": { "type": "string" }
        },
        "anyOf": [
            { "required": ["appId"] },
            { "required": ["bundle"] }
        ],
        "additionalProperties": false
    });
    let optional_app_arg = json!({
        "type": "object",
        "properties": {
            "appId": { "type": "string" },
            "bundle": { "type": "string" },
            "windowId": { "type": "string" }
        },
        "additionalProperties": false
    });
    let grant_arg = json!({
        "type": "object",
        "properties": {
            "permission": {
                "type": "string",
                "enum": ["screenshots", "accessibility"]
            }
        },
        "required": ["permission"],
        "additionalProperties": false
    });
    let list_apps_arg = json!({
        "type": "object",
        "properties": {
            "days": {
                "type": "integer",
                "minimum": 1,
                "description": "Recent-use ranking window in days. Providers use available OS activity metadata and fall back to running/name ordering when unavailable."
            }
        },
        "additionalProperties": false
    });
    let snapshot_arg = |extra: Value| {
        let mut props = json!({ "snapshotId": { "type": "string" } });
        if let (Some(obj), Some(extra_obj)) = (props.as_object_mut(), extra.as_object()) {
            for (k, v) in extra_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
        json!({ "type": "object", "properties": props, "required": ["snapshotId"], "additionalProperties": false })
    };
    let coord_space = json!({
        "type": "string",
        "enum": ["screenshot", "screen"],
        "default": "screenshot"
    });
    let element_dispatch_route = json!({
        "type": "string",
        "enum": ["auto", "ax", "target_pid", "hid"],
        "default": "auto"
    });
    let point_dispatch_route = json!({
        "type": "string",
        "enum": ["auto", "target_pid", "hid"],
        "default": "auto"
    });
    let point_action_arg = json!({
        "type": "object",
        "properties": {
            "snapshotId": { "type": "string" },
            "x": { "type": "number" },
            "y": { "type": "number" },
            "coordSpace": coord_space.clone(),
            "coord_space": coord_space.clone(),
            "dispatchRoute": point_dispatch_route.clone(),
            "dispatch_route": point_dispatch_route.clone()
        },
        "required": ["snapshotId", "x", "y"],
        "additionalProperties": false
    });
    let click_arg = json!({
        "type": "object",
        "properties": {
            "snapshotId": { "type": "string" },
            "elementId": { "type": "string" },
            "ref": { "type": "string" },
            "x": { "type": "number" },
            "y": { "type": "number" },
            "coordSpace": coord_space.clone(),
            "coord_space": coord_space.clone(),
            "dispatchRoute": element_dispatch_route.clone(),
            "dispatch_route": element_dispatch_route.clone()
        },
        "required": ["snapshotId"],
        "anyOf": [
            { "required": ["elementId"] },
            { "required": ["ref"] },
            { "required": ["x", "y"] }
        ],
        "additionalProperties": false
    });
    let drag_arg = json!({
        "type": "object",
        "properties": {
            "snapshotId": { "type": "string" },
            "fromX": { "type": "number" },
            "fromY": { "type": "number" },
            "toX": { "type": "number" },
            "toY": { "type": "number" },
            "coordSpace": coord_space.clone(),
            "coord_space": coord_space.clone(),
            "dispatchRoute": point_dispatch_route.clone(),
            "dispatch_route": point_dispatch_route.clone()
        },
        "required": ["snapshotId", "fromX", "fromY", "toX", "toY"],
        "additionalProperties": false
    });
    let secondary_arg = json!({
        "type": "object",
        "properties": {
            "snapshotId": { "type": "string" },
            "elementId": { "type": "string" },
            "ref": { "type": "string" },
            "x": { "type": "number" },
            "y": { "type": "number" },
            "coordSpace": coord_space.clone(),
            "coord_space": coord_space.clone(),
            "dispatchRoute": element_dispatch_route.clone(),
            "dispatch_route": element_dispatch_route.clone()
        },
        "required": ["snapshotId"],
        "anyOf": [
            { "required": ["elementId"] },
            { "required": ["ref"] },
            { "required": ["x", "y"] }
        ],
        "additionalProperties": false
    });
    vec![
        ToolDefinition {
            name: "computer_status",
            description: "Report whether computer use is available for this session and which capabilities (observe/inspect/control) are currently permitted.",
            input_schema: no_args.clone(),
        },
        ToolDefinition {
            name: "computer_permissions",
            description: "Report computer-use OS permission status, missing grants, and onboarding guidance.",
            input_schema: no_args.clone(),
        },
        ToolDefinition {
            name: "computer_grant",
            description: "Open the relevant OS settings page for a missing computer-use permission when supported.",
            input_schema: grant_arg,
        },
        ToolDefinition {
            name: "computer_list_apps",
            description: "List applications that can be targeted for computer use, ordered by recent usage when OS activity metadata is available. Results include running plus installed apps.",
            input_schema: list_apps_arg,
        },
        ToolDefinition {
            name: "computer_start",
            description: "Open a control lease on a chosen application/window.",
            input_schema: app_arg.clone(),
        },
        ToolDefinition {
            name: "computer_launch_app",
            description: "Launch a chosen application without activating it. Use for cold/background launch only; this does not reliably restore hidden or minimized windows. Requires target-app approval and opens a lease for the app.",
            input_schema: app_arg.clone(),
        },
        ToolDefinition {
            name: "computer_raise_app",
            description: "Foreground-recovery tool for hidden or minimized apps. Use this after computer_get_app_state reports no visible window, or before trying screenshots/clicks on a hidden target. Platform-specific fallback guidance is surfaced in tool errors; do not substitute shell/app-launcher shortcuts for this recovery path.",
            input_schema: app_arg,
        },
        ToolDefinition {
            name: "computer_get_app_state",
            description: "Capture the target's screenshot, display metadata, accessibility elements, a fresh snapshot id, and the actions currently allowed. If appId is provided and the app is not running, launch it after target-app approval. If this reports no visible window, call computer_raise_app for the same app target, then retry this tool.",
            input_schema: optional_app_arg,
        },
        ToolDefinition {
            name: "computer_click",
            description: "Click using the requested dispatchRoute from the latest snapshot. Supported action modes and routes vary by platform/provider; inspect AppState.actionCapabilities before choosing element vs point and dispatchRoute. Returns a post-action AppState plus lastClickResult { route, outcome, nextDispatchRoute? }.",
            input_schema: click_arg.clone(),
        },
        ToolDefinition {
            name: "computer_click_element",
            description: "Click an accessibility element from the latest snapshot. Treat the schema routes as the global upper bound; the currently supported subset is reported in AppState.actionCapabilities.clickElementRoutes. Returns a post-action AppState plus lastClickResult { route, outcome, nextDispatchRoute? }.",
            input_schema: snapshot_arg(json!({
                "elementId": { "type": "string" },
                "ref": { "type": "string" },
                "dispatchRoute": element_dispatch_route.clone(),
                "dispatch_route": element_dispatch_route.clone()
            })),
        },
        ToolDefinition {
            name: "computer_click_at",
            description: "Click by screenshot pixel from the latest snapshot. Treat the schema routes as the global upper bound; the currently supported subset is reported in AppState.actionCapabilities.clickAtRoutes. Returns a post-action AppState plus lastClickResult { route, outcome, nextDispatchRoute? }.",
            input_schema: point_action_arg.clone(),
        },
        ToolDefinition {
            name: "computer_secondary_click",
            description: "Secondary/right click or open a context menu. The schema routes are the global upper bound; inspect AppState.actionCapabilities.secondaryClickElementRoutes and secondaryClickAtRoutes for the currently supported subset. Returns a post-action AppState plus lastActionResult { kind, route, outcome }.",
            input_schema: secondary_arg,
        },
        ToolDefinition {
            name: "computer_perform_secondary_action",
            description: "Skill-compatible alias for computer_secondary_click. Use AppState.actionCapabilities to choose a supported element or point route before calling it.",
            input_schema: click_arg.clone(),
        },
        ToolDefinition {
            name: "computer_double_click",
            description: "Double click a point from the latest snapshot. Treat the schema routes as the global upper bound; the currently supported subset is reported in AppState.actionCapabilities.doubleClickAtRoutes. Returns a post-action AppState plus lastActionResult { kind, route, outcome }.",
            input_schema: point_action_arg.clone(),
        },
        ToolDefinition {
            name: "computer_drag",
            description: "Drag between two points from the latest snapshot. Treat the schema routes as the global upper bound; the currently supported subset is reported in AppState.actionCapabilities.dragRoutes. Returns a post-action AppState plus lastActionResult { kind, route, outcome }.",
            input_schema: drag_arg,
        },
        ToolDefinition {
            name: "computer_set_value",
            description: "Set an accessibility element's value directly from the latest snapshot. Check AppState.actionCapabilities.supportsSetValue before calling. Returns a post-action AppState plus lastActionResult { kind, route, outcome }.",
            input_schema: snapshot_arg(json!({
                "elementId": { "type": "string" },
                "ref": { "type": "string" },
                "value": { "type": "string" }
            })),
        },
        ToolDefinition {
            name: "computer_type_text",
            description: "Type text into the focused element of the latest snapshot. Check AppState.actionCapabilities.supportsTypeText before calling.",
            input_schema: snapshot_arg(json!({ "text": { "type": "string" } })),
        },
        ToolDefinition {
            name: "computer_press_key",
            description: "Press a named key or chord against the latest snapshot. Check AppState.actionCapabilities.supportsPressKey before calling.",
            input_schema: snapshot_arg(json!({ "key": { "type": "string" } })),
        },
        ToolDefinition {
            name: "computer_scroll",
            description: "Scroll the target in a direction by an amount against the latest snapshot. The schema routes are the global upper bound; inspect AppState.actionCapabilities.scrollElementRoutes and scrollAtRoutes for the currently supported subset. Returns a post-action AppState plus lastActionResult { kind, route, outcome }.",
            input_schema: snapshot_arg(json!({
                "elementId": { "type": "string" },
                "ref": { "type": "string" },
                "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
                "amount": { "type": "integer" },
                "dispatchRoute": element_dispatch_route.clone(),
                "dispatch_route": element_dispatch_route.clone()
            })),
        },
        ToolDefinition {
            name: "computer_stop",
            description: "Release the current control lease.",
            input_schema: no_args,
        },
    ]
}

/// Tool names, for quick membership checks.
pub const TOOL_DEFINITIONS: &[&str] = &[
    "computer_status",
    "computer_permissions",
    "computer_grant",
    "computer_list_apps",
    "computer_start",
    "computer_launch_app",
    "computer_raise_app",
    "computer_get_app_state",
    "computer_click",
    "computer_click_element",
    "computer_click_at",
    "computer_secondary_click",
    "computer_perform_secondary_action",
    "computer_double_click",
    "computer_drag",
    "computer_set_value",
    "computer_type_text",
    "computer_press_key",
    "computer_scroll",
    "computer_stop",
];

/// Build the `initialize` result advertising server capabilities.
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "sessio-computer-use", "version": "1" },
    })
}

/// Build the `tools/list` result.
pub fn tools_list_result() -> Value {
    let tools: Vec<Value> = tool_definitions()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// Wrap a tool's textual output into the MCP `tools/call` content shape.
pub fn tool_text_result(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }] })
}

/// Wrap JSON output with both machine-readable structured content and a text
/// fallback for MCP clients that only render textual tool content.
pub fn tool_json_result(value: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "structuredContent": value,
    })
}

/// Wrap app-state JSON and append the screenshot as inline MCP image content
/// when the screenshot handle points at a readable local file.
pub fn tool_app_state_result(value: Value) -> Value {
    let mut content = vec![json!({ "type": "text", "text": value.to_string() })];
    if let Some(instructions) = screenshot_instruction_text(&value) {
        content.push(json!({ "type": "text", "text": instructions }));
    }
    if let Some(image) = screenshot_image_content(&value) {
        content.push(image);
    }
    if let Some(reference) = screenshot_reference_image_content(&value) {
        content.push(reference);
    }
    json!({
        "content": content,
        "structuredContent": value,
    })
}

fn screenshot_image_content(value: &Value) -> Option<Value> {
    let screenshot = value.get("screenshot")?;
    let handle = screenshot_handle(screenshot)?;
    let format = screenshot_format(screenshot)?;
    let mime_type = image_mime_type_for_format(format)?;
    let bytes = std::fs::read(handle).ok()?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(json!({
        "type": "image",
        "data": data,
        "mimeType": mime_type,
        "detail": "original",
    }))
}

fn screenshot_reference_image_content(value: &Value) -> Option<Value> {
    let screenshot = value.get("screenshot")?;
    let handle = screenshot_handle(screenshot)?;
    let marker = screenshot_click_marker(screenshot);
    let bytes = render_reference_overlay_png(std::path::Path::new(handle), marker).ok()?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(json!({
        "type": "image",
        "data": data,
        "mimeType": "image/png",
        "detail": "original",
    }))
}

fn screenshot_instruction_text(value: &Value) -> Option<String> {
    let screenshot = value.get("screenshot")?;
    let width = screenshot.get("width")?.as_u64()?;
    let height = screenshot.get("height")?.as_u64()?;
    let marker = screenshot_click_marker(screenshot).map(|point| {
        format!(
            "Last click marker is centered at approximately ({:.0}, {:.0}) in that same pixel space.",
            point.x, point.y
        )
    });
    let mut lines = vec![
        format!("This screenshot is {width}x{height} pixels."),
        "All click coordinates must be in this original pixel space.".to_string(),
        "A second image is included with a 50px grid/ruler overlay in the same coordinate space."
            .to_string(),
    ];
    if let Some(marker_line) = marker {
        lines.push(marker_line);
    }
    Some(lines.join("\n"))
}

fn screenshot_handle(screenshot: &Value) -> Option<&str> {
    screenshot.get("handle")?.as_str()
}

fn screenshot_format(screenshot: &Value) -> Option<&str> {
    screenshot
        .get("format")
        .and_then(|value| value.as_str())
        .or(Some("png"))
}

fn image_mime_type_for_format(format: &str) -> Option<&'static str> {
    match format.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn screenshot_click_marker(screenshot: &Value) -> Option<Point> {
    let marker = screenshot.get("clickMarker")?;
    Some(Point {
        x: marker.get("x")?.as_f64()? as f32,
        y: marker.get("y")?.as_f64()? as f32,
    })
}

/// Wrap a tool error into the MCP `tools/call` error-content shape (isError).
pub fn tool_error_result(message: impl Into<String>) -> Value {
    json!({
        "content": [{ "type": "text", "text": message.into() }],
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_request() {
        let req = McpRequest::parse(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.id, json!(1));
        assert_eq!(req.params, json!({}));
    }

    #[test]
    fn rejects_malformed_body() {
        let err = McpRequest::parse("not json").unwrap_err();
        match err {
            McpResponse::Error { code, .. } => assert_eq!(code, -32700),
            _ => panic!("expected parse error"),
        }
    }

    #[test]
    fn rejects_missing_method() {
        let err = McpRequest::parse(r#"{"id":1}"#).unwrap_err();
        match err {
            McpResponse::Error { code, .. } => assert_eq!(code, -32600),
            _ => panic!("expected invalid request"),
        }
    }

    #[test]
    fn tools_list_includes_full_catalog() {
        let result = tools_list_result();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOL_DEFINITIONS.len());
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in TOOL_DEFINITIONS {
            assert!(names.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn raise_tool_description_warns_against_unreliable_shell_recovery() {
        let result = tools_list_result();
        let tools = result["tools"].as_array().unwrap();
        let raise = tools
            .iter()
            .find(|tool| tool["name"] == "computer_raise_app")
            .expect("raise tool");
        let description = raise["description"].as_str().unwrap();
        assert!(description.contains("hidden or minimized"));
        assert!(description.contains("computer_get_app_state"));
        assert!(description.contains("tool errors"));
        assert!(!description.contains("open -a"));
    }

    #[test]
    fn click_tool_descriptions_point_to_action_capabilities() {
        let result = tools_list_result();
        let tools = result["tools"].as_array().unwrap();
        let click = tools
            .iter()
            .find(|tool| tool["name"] == "computer_click")
            .expect("click tool");
        let scroll = tools
            .iter()
            .find(|tool| tool["name"] == "computer_scroll")
            .expect("scroll tool");
        assert!(click["description"]
            .as_str()
            .unwrap()
            .contains("AppState.actionCapabilities"));
        assert!(scroll["description"]
            .as_str()
            .unwrap()
            .contains("AppState.actionCapabilities"));
    }

    #[test]
    fn response_serializes_jsonrpc_envelope() {
        let ok = McpResponse::result(json!(7), json!({"a":1})).to_json();
        assert_eq!(ok["jsonrpc"], "2.0");
        assert_eq!(ok["id"], 7);
        assert_eq!(ok["result"]["a"], 1);

        let err = McpResponse::error(json!(8), -32601, "method not found").to_json();
        assert_eq!(err["error"]["code"], -32601);
        assert_eq!(err["error"]["message"], "method not found");
    }

    #[test]
    fn json_tool_result_has_structured_content_and_text_fallback() {
        let result = tool_json_result(json!({ "snapshotId": "s1" }));
        assert_eq!(result["structuredContent"]["snapshotId"], "s1");
        assert_eq!(
            result["content"][0]["text"].as_str().unwrap(),
            r#"{"snapshotId":"s1"}"#
        );
    }

    #[test]
    fn app_state_result_inlines_readable_screenshot_image() {
        let path = std::env::temp_dir().join(format!(
            "sessio-mcp-image-test-{}-{}.png",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let image =
            image::RgbaImage::from_pixel(120, 80, image::Rgba([20_u8, 20_u8, 20_u8, 255_u8]));
        image.save(&path).unwrap();
        let result = tool_app_state_result(json!({
            "snapshotId": "snap-1",
            "screenshot": {
                "handle": path.to_string_lossy(),
                "format": "png",
                "width": 120,
                "height": 80
            }
        }));
        let _ = std::fs::remove_file(&path);

        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][1]["type"], "text");
        assert!(result["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("This screenshot is 120x80 pixels."));
        assert_eq!(result["content"][2]["type"], "image");
        assert_eq!(result["content"][2]["mimeType"], "image/png");
        assert_eq!(result["content"][2]["detail"], "original");
        assert_eq!(result["content"][3]["type"], "image");
        assert_eq!(result["content"][3]["mimeType"], "image/png");
        assert_eq!(result["content"][3]["detail"], "original");
    }

    #[test]
    fn app_state_result_includes_click_marker_in_instruction_text() {
        let result = tool_app_state_result(json!({
            "snapshotId": "snap-1",
            "screenshot": {
                "handle": "/does/not/exist.png",
                "format": "png",
                "width": 1056,
                "height": 880,
                "clickMarker": {
                    "x": 383.0,
                    "y": 395.0
                }
            }
        }));

        let text = result["content"][1]["text"].as_str().unwrap();
        assert!(text.contains("This screenshot is 1056x880 pixels."));
        assert!(text.contains("All click coordinates must be in this original pixel space."));
        assert!(text.contains("50px grid/ruler overlay"));
        assert!(text.contains("(383, 395)"));
    }

    #[test]
    fn app_state_result_text_fallback_preserves_last_click_result() {
        let result = tool_app_state_result(json!({
            "snapshotId": "snap-1",
            "lastClickResult": {
                "route": "hid",
                "outcome": "no_effect",
                "nextDispatchRoute": "hid"
            },
            "screenshot": {
                "handle": "/does/not/exist.png",
                "format": "png"
            }
        }));

        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"lastClickResult\""));
        assert!(text.contains("\"route\":\"hid\""));
        assert!(text.contains("\"outcome\":\"no_effect\""));
        assert!(text.contains("\"nextDispatchRoute\":\"hid\""));
        assert_eq!(
            result["structuredContent"]["lastClickResult"]["route"],
            "hid"
        );
        assert_eq!(
            result["structuredContent"]["lastClickResult"]["outcome"],
            "no_effect"
        );
        assert_eq!(
            result["structuredContent"]["lastClickResult"]["nextDispatchRoute"],
            "hid"
        );
    }

    #[test]
    fn app_state_result_text_fallback_preserves_last_action_result() {
        let result = tool_app_state_result(json!({
            "snapshotId": "snap-1",
            "lastActionResult": {
                "kind": "scroll",
                "route": "ax",
                "outcome": "semantic_success"
            },
            "screenshot": {
                "handle": "/does/not/exist.png",
                "format": "png"
            }
        }));

        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"lastActionResult\""));
        assert!(text.contains("\"kind\":\"scroll\""));
        assert!(text.contains("\"route\":\"ax\""));
        assert!(text.contains("\"outcome\":\"semantic_success\""));
        assert_eq!(
            result["structuredContent"]["lastActionResult"]["kind"],
            "scroll"
        );
        assert_eq!(
            result["structuredContent"]["lastActionResult"]["route"],
            "ax"
        );
        assert_eq!(
            result["structuredContent"]["lastActionResult"]["outcome"],
            "semantic_success"
        );
    }

    #[test]
    fn app_state_result_text_fallback_preserves_action_capabilities() {
        let result = tool_app_state_result(json!({
            "snapshotId": "snap-1",
            "actionCapabilities": {
                "clickElementRoutes": ["auto"],
                "clickAtRoutes": ["auto"],
                "secondaryClickElementRoutes": [],
                "secondaryClickAtRoutes": ["auto"],
                "doubleClickAtRoutes": ["auto"],
                "dragRoutes": ["auto"],
                "scrollElementRoutes": [],
                "scrollAtRoutes": ["auto"],
                "supportsSetValue": true,
                "supportsTypeText": true,
                "supportsPressKey": true
            },
            "screenshot": {
                "handle": "/does/not/exist.png",
                "format": "png"
            }
        }));

        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"actionCapabilities\""));
        assert!(text.contains("\"clickElementRoutes\":[\"auto\"]"));
        assert_eq!(
            result["structuredContent"]["actionCapabilities"]["scrollElementRoutes"],
            json!([])
        );
    }

    #[test]
    fn initialize_advertises_tools_capability() {
        let init = initialize_result();
        assert!(init["capabilities"]["tools"].is_object());
        assert_eq!(init["serverInfo"]["name"], "sessio-computer-use");
    }
}
