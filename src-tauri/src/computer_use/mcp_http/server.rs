//! `tiny_http`-backed lifecycle for the desktop-owned MCP server.
//!
//! Binds `127.0.0.1:0` (ephemeral loopback port), serves MCP JSON-RPC over HTTP
//! POST on a dedicated thread, and shuts down cleanly when the handle is
//! dropped. Each request is authenticated via [`super::auth::TokenRegistry`]
//! (bearer token → session id, loopback-enforced) and dispatched into the
//! [`ComputerUseHost`] for that session.
//!
//! One server instance is shared process-wide; per-session isolation comes from
//! the token→session mapping and the host's own per-session lease/approval
//! state. This matches v3's "one shared server with per-session routing" option.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::computer_use::host::ComputerUseHost;
use crate::desktop_control::DesktopControlPermissionStatus;

use super::auth::{SessionToken, TokenRegistry};
use super::dispatch::dispatch;
use super::protocol::{McpRequest, McpResponse};

/// Callback that returns the current desktop-control permission status. Injected
/// so the server reflects live OS permission state on every request without the
/// HTTP layer depending on the platform FFI directly.
pub type PermissionProvider = Arc<dyn Fn() -> DesktopControlPermissionStatus + Send + Sync>;

/// A running MCP HTTP server. Dropping the handle stops the server thread.
pub struct McpServerHandle {
    addr: SocketAddr,
    tokens: Arc<TokenRegistry>,
    server: Arc<tiny_http::Server>,
    thread: Option<JoinHandle<()>>,
}

impl McpServerHandle {
    /// The loopback base URL agents should connect to (e.g. `http://127.0.0.1:54321`).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The MCP endpoint URL injected into `session/new`.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url())
    }

    /// Issue a bearer token for a session. Inject it as an `Authorization:
    /// Bearer <token>` header on the `McpServer::Http` entry.
    pub fn issue_token(&self, session_id: &str) -> SessionToken {
        self.tokens.issue(session_id)
    }

    /// Revoke a session's token so its endpoint stops working (session end /
    /// recreate / transport failure).
    pub fn revoke_session(&self, session_id: &str) {
        self.tokens.revoke_session(session_id);
    }
}

impl Drop for McpServerHandle {
    fn drop(&mut self) {
        // `unblock` wakes the blocking `recv()` so the worker thread exits.
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Builder/entry point for the server.
pub struct McpHttpServer;

impl McpHttpServer {
    /// Start the server on an ephemeral loopback port.
    ///
    /// `host` resolves a session id to its [`ComputerUseHost`] (one host may be
    /// shared by all sessions, or per-session — the closure decides).
    pub fn start(
        host_for_session: Arc<dyn Fn(&str) -> Option<ComputerUseHost> + Send + Sync>,
        permission_provider: PermissionProvider,
    ) -> std::io::Result<McpServerHandle> {
        let bind = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let server = tiny_http::Server::http(bind)
            .map_err(|e| std::io::Error::other(format!("tiny_http bind failed: {e}")))?;
        let server = Arc::new(server);
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| std::io::Error::other("server address is not IP"))?;
        let tokens = Arc::new(TokenRegistry::new());

        let worker_server = server.clone();
        let worker_tokens = tokens.clone();
        let thread = std::thread::Builder::new()
            .name("sessio-computer-use-mcp".into())
            .spawn(move || {
                serve_loop(
                    worker_server,
                    worker_tokens,
                    host_for_session,
                    permission_provider,
                );
            })?;

        Ok(McpServerHandle {
            addr,
            tokens,
            server,
            thread: Some(thread),
        })
    }
}

fn serve_loop(
    server: Arc<tiny_http::Server>,
    tokens: Arc<TokenRegistry>,
    host_for_session: Arc<dyn Fn(&str) -> Option<ComputerUseHost> + Send + Sync>,
    permission_provider: PermissionProvider,
) {
    loop {
        let request = match server.recv() {
            Ok(req) => req,
            // `unblock()` (on drop) or a fatal error breaks the loop.
            Err(_) => break,
        };
        handle_request(request, &tokens, &host_for_session, &permission_provider);
    }
}

fn handle_request(
    mut request: tiny_http::Request,
    tokens: &TokenRegistry,
    host_for_session: &Arc<dyn Fn(&str) -> Option<ComputerUseHost> + Send + Sync>,
    permission_provider: &PermissionProvider,
) {
    let is_loopback = request
        .remote_addr()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or(false);
    let auth_header = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .map(|h| h.value.as_str().to_string());

    // Resolve the session from the token (loopback-enforced).
    let session_id = match tokens.resolve(auth_header.as_deref(), is_loopback) {
        Ok(sid) => sid,
        Err(err) => {
            respond_unauthorized(request, &err.to_string());
            return;
        }
    };

    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        respond_json(
            request,
            &McpResponse::error(serde_json::Value::Null, -32700, "could not read body")
                .to_string(),
        );
        return;
    }

    let parsed = match McpRequest::parse(&body) {
        Ok(req) => req,
        Err(error_response) => {
            respond_json(request, &error_response.to_string());
            return;
        }
    };

    let Some(host) = host_for_session(&session_id) else {
        respond_json(
            request,
            &McpResponse::error(parsed.id.clone(), -32000, "session is no longer active")
                .to_string(),
        );
        return;
    };

    let perm = permission_provider();
    let response = dispatch(&host, &session_id, &perm, &parsed);
    respond_json(request, &response.to_string());
}

fn respond_json(request: tiny_http::Request, body: &str) {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("valid content-type header");
    let response = tiny_http::Response::from_string(body).with_header(header);
    let _ = request.respond(response);
}

fn respond_unauthorized(request: tiny_http::Request, message: &str) {
    let response = tiny_http::Response::from_string(message).with_status_code(401);
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::provider::FakeProvider;
    use crate::computer_use::settings::ComputerUseSettings;
    use crate::desktop_control::{
        DesktopControlInputs, DesktopControlPermissionStatus, DesktopPlatform, PermissionTier,
    };

    fn full_perm() -> DesktopControlPermissionStatus {
        DesktopControlPermissionStatus::derive(DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(true, true),
            accessibility: PermissionTier::new(true, true),
            input_injection_supported: true,
        })
    }

    fn test_host() -> ComputerUseHost {
        ComputerUseHost::new(
            Arc::new(FakeProvider::default()),
            ComputerUseSettings::recommended(),
        )
    }

    fn start_server() -> (McpServerHandle, ComputerUseHost) {
        let host = test_host();
        let host_clone = host.clone();
        let handle = McpHttpServer::start(
            Arc::new(move |_sid: &str| Some(host_clone.clone())),
            Arc::new(full_perm),
        )
        .expect("server starts");
        (handle, host)
    }

    fn post(url: &str, token: Option<&str>, body: &str) -> (u16, String) {
        let client = reqwest::blocking::Client::new();
        let mut req = client.post(url).body(body.to_string());
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let resp = req.send().expect("request sends");
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        (status, text)
    }

    #[test]
    fn binds_to_loopback() {
        let (handle, _host) = start_server();
        assert!(handle.base_url().starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn rejects_request_without_token() {
        let (handle, _host) = start_server();
        let (status, _) = post(&handle.mcp_url(), None, r#"{"id":1,"method":"tools/list"}"#);
        assert_eq!(status, 401);
    }

    #[test]
    fn rejects_unknown_token() {
        let (handle, _host) = start_server();
        let (status, _) = post(
            &handle.mcp_url(),
            Some("deadbeef"),
            r#"{"id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 401);
    }

    #[test]
    fn authed_tools_list_returns_catalog() {
        let (handle, _host) = start_server();
        let token = handle.issue_token("s1");
        let (status, body) = post(
            &handle.mcp_url(),
            Some(&token.0),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(json["result"]["tools"].as_array().unwrap().len() >= 9);
    }

    #[test]
    fn revoked_token_stops_working() {
        let (handle, _host) = start_server();
        let token = handle.issue_token("s1");
        handle.revoke_session("s1");
        let (status, _) = post(
            &handle.mcp_url(),
            Some(&token.0),
            r#"{"id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 401);
    }

    #[test]
    fn status_tool_round_trips_over_http() {
        let (handle, _host) = start_server();
        let token = handle.issue_token("s1");
        let (status, body) = post(
            &handle.mcp_url(),
            Some(&token.0),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"computer_status","arguments":{}}}"#,
        );
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = json["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["enabled"], true); // observe_only host is enabled
    }
}
