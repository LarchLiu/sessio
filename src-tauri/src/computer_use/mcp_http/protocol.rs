//! MCP JSON-RPC protocol framing for the computer-use tool server.
//!
//! Implements the subset of the Model Context Protocol the ACP adapters use to
//! discover and invoke tools over HTTP: `initialize`, `tools/list`, and
//! `tools/call`. Parsing/serialization is pure so it is unit-testable without a
//! socket; dispatch of `tools/call` into the host lives in [`super::dispatch`].

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
        "properties": { "appId": { "type": "string" }, "windowId": { "type": "string" } },
        "required": ["appId"],
        "additionalProperties": false
    });
    let optional_app_arg = json!({
        "type": "object",
        "properties": { "appId": { "type": "string" }, "windowId": { "type": "string" } },
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
    vec![
        ToolDefinition {
            name: "computer_status",
            description: "Report whether computer use is available for this session and which capabilities (observe/inspect/control) are currently permitted.",
            input_schema: no_args.clone(),
        },
        ToolDefinition {
            name: "computer_list_apps",
            description: "List applications that can be targeted for computer use.",
            input_schema: no_args.clone(),
        },
        ToolDefinition {
            name: "computer_start",
            description: "Open a control lease on a chosen application/window.",
            input_schema: app_arg.clone(),
        },
        ToolDefinition {
            name: "computer_launch_app",
            description: "Launch a chosen application without activating it. Requires target-app approval and opens a lease for the app.",
            input_schema: app_arg,
        },
        ToolDefinition {
            name: "computer_get_app_state",
            description: "Capture the target's screenshot, display metadata, accessibility elements, a fresh snapshot id, and the actions currently allowed. If appId is provided and the app is not running, launch it after target-app approval.",
            input_schema: optional_app_arg,
        },
        ToolDefinition {
            name: "computer_click_element",
            description: "Click an accessibility element from the latest snapshot.",
            input_schema: snapshot_arg(json!({ "elementId": { "type": "string" } })),
        },
        ToolDefinition {
            name: "computer_type_text",
            description: "Type text into the focused element of the latest snapshot.",
            input_schema: snapshot_arg(json!({ "text": { "type": "string" } })),
        },
        ToolDefinition {
            name: "computer_press_key",
            description: "Press a named key or chord against the latest snapshot.",
            input_schema: snapshot_arg(json!({ "key": { "type": "string" } })),
        },
        ToolDefinition {
            name: "computer_scroll",
            description: "Scroll the target in a direction by an amount against the latest snapshot.",
            input_schema: snapshot_arg(json!({
                "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
                "amount": { "type": "integer" }
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
    "computer_list_apps",
    "computer_start",
    "computer_launch_app",
    "computer_get_app_state",
    "computer_click_element",
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
    fn initialize_advertises_tools_capability() {
        let init = initialize_result();
        assert!(init["capabilities"]["tools"].is_object());
        assert_eq!(init["serverInfo"]["name"], "sessio-computer-use");
    }
}
