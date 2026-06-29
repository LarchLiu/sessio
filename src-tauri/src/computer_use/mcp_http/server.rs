//! `rmcp` Streamable HTTP lifecycle for the desktop-owned MCP server and attach broker.
//!
//! Binds `127.0.0.1:0` (ephemeral loopback port), serves MCP Streamable HTTP
//! through the official Rust SDK, and shuts down cleanly when the handle is
//! dropped. Each MCP request is authenticated via [`super::auth::TokenRegistry`]
//! (bearer token → session id) and dispatched into the [`ComputerUseHost`] for
//! that session.
//!
//! One server instance is shared process-wide; per-session isolation comes from
//! the token→session mapping and the host's own per-session lease/approval
//! state. This matches v3's "one shared server with per-session routing" option.

use std::borrow::Cow;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};
use std::thread::JoinHandle;

use axum::{
    body::Bytes,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use rmcp::{
    handler::server::ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, Content, Implementation, InitializeResult,
        JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
    },
    service::{RequestContext, RoleServer},
    transport::streamable_http_server::{
        session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData,
};

use crate::computer_use::host::ComputerUseHost;
use crate::desktop_control::DesktopControlPermissionStatus;

use super::super::broker::{self, ExternalAttachRequest, ExternalAttachResponse};
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
    shutdown: Option<mpsc::Sender<()>>,
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

    /// The external attach broker URL published through the discovery file.
    pub fn attach_url(&self) -> String {
        format!("{}/attach", self.base_url())
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
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
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
        let listener = TcpListener::bind(bind)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let tokens = Arc::new(TokenRegistry::new());
        let external_session_counter = Arc::new(AtomicU64::new(1));

        let worker_tokens = tokens.clone();
        let broker_base_url = format!("http://{}", addr);
        let mcp_url = format!("{broker_base_url}/mcp");
        if let Err(error) = broker::write_discovery(broker_base_url.clone(), mcp_url.clone()) {
            log::warn!("[computer-use:broker] failed to publish discovery file: {error}");
        } else {
            log::info!("[computer-use:broker] published discovery at {broker_base_url}");
        }
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("sessio-computer-use-broker".into())
            .spawn(move || {
                if let Err(error) = serve_loop(
                    listener,
                    worker_tokens,
                    external_session_counter,
                    mcp_url,
                    host_for_session,
                    permission_provider,
                    shutdown_rx,
                ) {
                    log::error!("[computer-use:broker] server stopped with error: {error}");
                }
            })?;

        Ok(McpServerHandle {
            addr,
            tokens,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        })
    }
}

fn serve_loop(
    listener: TcpListener,
    tokens: Arc<TokenRegistry>,
    external_session_counter: Arc<AtomicU64>,
    mcp_url: String,
    host_for_session: Arc<dyn Fn(&str) -> Option<ComputerUseHost> + Send + Sync>,
    permission_provider: PermissionProvider,
    shutdown_rx: mpsc::Receiver<()>,
) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| std::io::Error::other(format!("tokio runtime failed: {error}")))?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let state = BrokerState {
            tokens: tokens.clone(),
            external_session_counter,
            mcp_url,
            host_for_session: host_for_session.clone(),
        };
        let mcp_service = SessioComputerUseService {
            tokens,
            host_for_session,
            permission_provider,
        };
        let sdk_service: StreamableHttpService<SessioComputerUseService, NeverSessionManager> =
            StreamableHttpService::new(
                move || Ok(mcp_service.clone()),
                Default::default(),
                StreamableHttpServerConfig::default()
                    .with_stateful_mode(false)
                    .with_json_response(true)
                    .with_sse_keep_alive(None),
            );
        let app = Router::new()
            .nest_service("/mcp", sdk_service)
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_mcp_auth,
            ))
            .route("/attach", post(handle_attach))
            .with_state(state);

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = tokio::task::spawn_blocking(move || shutdown_rx.recv()).await;
        })
        .await
        .map_err(|error| std::io::Error::other(format!("axum serve failed: {error}")))
    })
}

#[derive(Clone)]
struct BrokerState {
    tokens: Arc<TokenRegistry>,
    external_session_counter: Arc<AtomicU64>,
    mcp_url: String,
    host_for_session: Arc<dyn Fn(&str) -> Option<ComputerUseHost> + Send + Sync>,
}

#[derive(Clone)]
struct SessioComputerUseService {
    tokens: Arc<TokenRegistry>,
    host_for_session: Arc<dyn Fn(&str) -> Option<ComputerUseHost> + Send + Sync>,
    permission_provider: PermissionProvider,
}

impl SessioComputerUseService {
    fn resolve_session(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<(String, ComputerUseHost), ErrorData> {
        let auth_header = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| authorization_header(&parts.headers));
        let session_id = self
            .tokens
            .resolve(auth_header.as_deref(), true)
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let host = (self.host_for_session)(&session_id).ok_or_else(|| {
            ErrorData::internal_error("session is no longer active".to_string(), None)
        })?;
        Ok((session_id, host))
    }
}

impl ServerHandler for SessioComputerUseService {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("sessio-computer-use", "1"))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let authorized = self.resolve_session(&context).map(|_| ());
        std::future::ready(authorized.map(|_| {
            let mut result = ListToolsResult::default();
            result.tools = super::protocol::tool_definitions()
                .into_iter()
                .map(|definition| {
                    let mut tool = Tool::default();
                    tool.name = Cow::Borrowed(definition.name);
                    tool.description = Some(Cow::Borrowed(definition.description));
                    tool.input_schema = Arc::new(json_schema_object(definition.input_schema));
                    tool
                })
                .collect();
            result
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
        let resolved = self.resolve_session(&context);
        async move {
            let (session_id, host) = resolved?;
            let params = serde_json::json!({
                "name": request.name.as_ref(),
                "arguments": request.arguments.unwrap_or_default(),
            });
            let request = McpRequest {
                id: serde_json::Value::Null,
                method: "tools/call".into(),
                params,
            };
            match dispatch(&host, &session_id, &(self.permission_provider)(), &request) {
                McpResponse::Result { result, .. } => {
                    serde_json::from_value::<CallToolResult>(result).map_err(|error| {
                        ErrorData::internal_error(
                            format!("invalid tool result from computer-use dispatcher: {error}"),
                            None,
                        )
                    })
                }
                McpResponse::Error { message, .. } => {
                    Ok(CallToolResult::error(vec![Content::text(message)]))
                }
            }
        }
    }
}

async fn handle_attach(
    State(state): State<BrokerState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    if !remote_addr.ip().is_loopback() {
        return (
            StatusCode::UNAUTHORIZED,
            "attach is limited to loopback clients",
        )
            .into_response();
    }
    let attach: ExternalAttachRequest = if body.is_empty() {
        ExternalAttachRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": format!("invalid attach request: {error}")
                    })),
                )
                    .into_response();
            }
        }
    };
    let sequence = state
        .external_session_counter
        .fetch_add(1, Ordering::Relaxed);
    let label = attach
        .client_name
        .as_deref()
        .map(sanitize_session_part)
        .filter(|part| !part.is_empty())
        .unwrap_or_else(|| "client".into());
    let session_id = format!("external-{label}-{}-{sequence}", std::process::id());
    let Some(host) = (state.host_for_session)(&session_id) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "computer-use host is unavailable" })),
        )
            .into_response();
    };
    host.approvals().approve_session(&session_id);
    let token = state.tokens.issue(&session_id);
    let response = ExternalAttachResponse {
        schema_version: 1,
        session_id,
        mcp_url: state.mcp_url.clone(),
        token: token.0,
    };
    axum::Json(response).into_response()
}

async fn require_mcp_auth(
    State(state): State<BrokerState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let auth_header = authorization_header(&headers);
    let is_loopback = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|addr| addr.0.ip().is_loopback())
        .unwrap_or(false);
    match state.tokens.resolve(auth_header.as_deref(), is_loopback) {
        Ok(_) => next.run(request).await,
        Err(error) => (StatusCode::UNAUTHORIZED, error.to_string()).into_response(),
    }
}

fn authorization_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn json_schema_object(value: serde_json::Value) -> JsonObject {
    match value {
        serde_json::Value::Object(map) => map,
        _ => JsonObject::default(),
    }
}

fn sanitize_session_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase()
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
        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(body.to_string());
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let resp = req.send().expect("request sends");
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        (status, text)
    }

    fn attach(url: &str, body: &str) -> (u16, String) {
        let resp = reqwest::blocking::Client::new()
            .post(format!("{}/attach", url.trim_end_matches('/')))
            .body(body.to_string())
            .send()
            .expect("attach request sends");
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
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
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
    fn attach_issues_external_session_token() {
        let (handle, host) = start_server();
        let (status, body) = attach(
            &handle.base_url(),
            r#"{"clientName":"External Agent","clientPid":42}"#,
        );
        assert_eq!(status, 200);
        let attach: ExternalAttachResponse = serde_json::from_str(&body).unwrap();
        assert!(attach.session_id.starts_with("external-external-agent-"));
        assert_eq!(attach.mcp_url, handle.mcp_url());
        assert!(!attach.token.is_empty());
        assert!(host.approvals().session_approved(&attach.session_id));

        let (status, body) = post(
            &attach.mcp_url,
            Some(&attach.token),
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
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
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
