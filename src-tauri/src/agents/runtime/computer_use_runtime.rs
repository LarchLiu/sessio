//! Runtime glue for `computer use`: decides eligibility, owns the shared
//! desktop MCP server + host, and produces per-session injections.
//!
//! This is the shape-independent runtime layer between the chat option and the
//! ACP `session/new` injection. It:
//!
//! - parses the `computerUse` session option,
//! - gates ACP injection on `mcp_injection.http` and Pi injection on
//!   `mcp_injection.native_extension`,
//! - owns one desktop-started HTTP MCP server / attach broker (loopback),
//! - issues a per-session bearer token and hands back a [`ComputerUseInjection`]
//!   for ACP `new_session_request` or Pi extension environment injection,
//! - revokes the token on session teardown.
//!
//! Sessio starts the broker at app startup so external `sessio cu` clients can
//! discover it without first creating an in-app agent session. Ordinary sessions
//! still do not receive a token unless they explicitly request computer use.

use std::sync::{Arc, Mutex, OnceLock};

use crate::computer_use::host::ComputerUseHost;
use crate::computer_use::mcp_http::{McpHttpServer, McpServerHandle};
use crate::computer_use::pointer_overlay;
use crate::computer_use::settings::ComputerUseSettings;
use crate::desktop_control::DesktopControlPermissionStatus;
use tauri::AppHandle;

use super::types::{ComputerUseInjection, RuntimeCapabilitySet, RuntimeMetadata};

/// Parse the `computerUse` boolean session option (default false).
pub fn computer_use_requested(options: &RuntimeMetadata) -> bool {
    options
        .get("computerUse")
        .or_else(|| options.get("computer_use"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Whether the session may inject computer use: requested AND the agent's
/// capabilities advertise HTTP MCP injection.
///
/// Per plan v3, the MVP gates on `mcp_injection.http`; `acp` does not gate.
pub fn should_inject(
    options: &RuntimeMetadata,
    capabilities: Option<&RuntimeCapabilitySet>,
) -> bool {
    if !computer_use_requested(options) {
        return false;
    }
    capabilities
        .map(|caps| caps.mcp_injection.http)
        .unwrap_or(false)
}

/// Whether a native agent extension (currently Pi) should be activated for this
/// session.
pub fn should_inject_native_extension(
    options: &RuntimeMetadata,
    capabilities: Option<&RuntimeCapabilitySet>,
) -> bool {
    if !computer_use_requested(options) {
        return false;
    }
    capabilities
        .map(|caps| caps.mcp_injection.native_extension)
        .unwrap_or(false)
}

/// Owns the desktop-side computer-use server + host, shared process-wide.
///
/// Provided to the runtime manager; sessions ask it for an injection.
pub struct ComputerUseRuntime {
    host: ComputerUseHost,
    permission_provider: Arc<dyn Fn() -> DesktopControlPermissionStatus + Send + Sync>,
    server: OnceLock<std::io::Result<McpServerHandle>>,
    // Guards lazy init so two sessions racing on first use don't double-start.
    init_lock: Mutex<()>,
}

impl ComputerUseRuntime {
    /// Build the runtime with a permission provider (live OS status callback).
    /// `settings` is the host enable policy.
    pub fn new(
        app: AppHandle,
        settings: ComputerUseSettings,
        permission_provider: Arc<dyn Fn() -> DesktopControlPermissionStatus + Send + Sync>,
    ) -> Self {
        let host = ComputerUseHost::with_platform_provider(settings.clone())
            .with_pointer_event_sink(pointer_overlay::tauri_pointer_event_sink(app));
        Self::from_host(host, settings, permission_provider)
    }

    #[cfg(test)]
    fn new_for_tests(
        settings: ComputerUseSettings,
        permission_provider: Arc<dyn Fn() -> DesktopControlPermissionStatus + Send + Sync>,
    ) -> Self {
        let host = ComputerUseHost::with_platform_provider(settings.clone());
        Self::from_host(host, settings, permission_provider)
    }

    fn from_host(
        host: ComputerUseHost,
        settings: ComputerUseSettings,
        permission_provider: Arc<dyn Fn() -> DesktopControlPermissionStatus + Send + Sync>,
    ) -> Self {
        host.approvals()
            .set_approved_apps(settings.approved_apps.clone());
        Self {
            host,
            permission_provider,
            server: OnceLock::new(),
            init_lock: Mutex::new(()),
        }
    }

    pub fn host(&self) -> &ComputerUseHost {
        &self.host
    }

    pub fn update_settings(&self, settings: ComputerUseSettings) {
        self.host
            .approvals()
            .set_approved_apps(settings.approved_apps.clone());
        self.host.update_settings(settings);
    }

    /// Lazily start (once) and return the shared MCP server handle.
    pub fn server(&self) -> Result<&McpServerHandle, String> {
        // Fast path: already initialized.
        if let Some(result) = self.server.get() {
            return result.as_ref().map_err(|e| e.to_string());
        }
        let _guard = self.init_lock.lock().unwrap();
        // Re-check after acquiring the lock.
        if self.server.get().is_none() {
            let host = self.host.clone();
            let provider = self.permission_provider.clone();
            let started = McpHttpServer::start(
                Arc::new(move |_session_id: &str| Some(host.clone())),
                provider,
            );
            // Ignore the set error: another thread may have set it first; we
            // read the stored value below regardless.
            let _ = self.server.set(started);
        }
        self.server
            .get()
            .expect("server initialized")
            .as_ref()
            .map_err(|e| e.to_string())
    }

    /// Prepare an injection for an eligible session: ensure the server is up,
    /// issue a bearer token, and return the URL + token to attach to
    /// `session/new`.
    pub fn prepare_injection(&self, session_id: &str) -> Result<ComputerUseInjection, String> {
        let server = self.server()?;
        let token = server.issue_token(session_id);
        Ok(ComputerUseInjection {
            url: server.mcp_url(),
            bearer_token: token.0,
        })
    }

    /// Tear down a session's computer-use state: revoke its token and release
    /// any lease/approval. Idempotent.
    pub fn teardown_session(&self, session_id: &str) {
        if let Some(Ok(server)) = self.server.get() {
            server.revoke_session(session_id);
        }
        self.host.stop(session_id);
        self.host.approvals().revoke_session(session_id);
        crate::computer_use::pointer_overlay::release_session(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::runtime::types::McpInjectionCapabilities;
    use crate::desktop_control::{DesktopControlInputs, DesktopPlatform, PermissionTier};
    use serde_json::json;

    fn options(value: serde_json::Value) -> RuntimeMetadata {
        let mut m = RuntimeMetadata::new();
        if let Some(b) = value.as_bool() {
            m.insert("computerUse".into(), json!(b));
        }
        m
    }

    fn caps(http: bool) -> RuntimeCapabilitySet {
        let mut c = RuntimeCapabilitySet::fake();
        c.mcp_injection = McpInjectionCapabilities {
            http,
            sse: false,
            acp: false,
            native_extension: false,
        };
        c
    }

    fn perm() -> DesktopControlPermissionStatus {
        DesktopControlPermissionStatus::derive(DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(true, true),
            accessibility: PermissionTier::new(true, true),
            input_injection_supported: false,
        })
    }

    #[test]
    fn requested_parses_both_casings_and_defaults_false() {
        assert!(!computer_use_requested(&RuntimeMetadata::new()));
        assert!(computer_use_requested(&options(json!(true))));
        let mut snake = RuntimeMetadata::new();
        snake.insert("computer_use".into(), json!(true));
        assert!(computer_use_requested(&snake));
    }

    #[test]
    fn should_inject_requires_request_and_http_capability() {
        // Requested but no http capability.
        assert!(!should_inject(&options(json!(true)), Some(&caps(false))));
        // http capable but not requested.
        assert!(!should_inject(&RuntimeMetadata::new(), Some(&caps(true))));
        // Both → inject.
        assert!(should_inject(&options(json!(true)), Some(&caps(true))));
        // No capabilities probed.
        assert!(!should_inject(&options(json!(true)), None));
    }

    #[test]
    fn native_extension_injection_requires_request_and_native_capability() {
        let mut native = caps(false);
        native.mcp_injection.native_extension = true;

        assert!(should_inject_native_extension(
            &options(json!(true)),
            Some(&native)
        ));
        assert!(!should_inject_native_extension(
            &RuntimeMetadata::new(),
            Some(&native)
        ));
        assert!(!should_inject_native_extension(
            &options(json!(true)),
            Some(&caps(false))
        ));
        assert!(!should_inject_native_extension(&options(json!(true)), None));
    }

    fn runtime() -> ComputerUseRuntime {
        ComputerUseRuntime::new_for_tests(ComputerUseSettings::enabled(), Arc::new(perm))
    }

    #[test]
    fn prepare_injection_starts_server_and_issues_token() {
        let rt = runtime();
        let injection = rt.prepare_injection("s1").expect("injection");
        assert!(injection.url.starts_with("http://127.0.0.1:"));
        assert!(injection.url.ends_with("/mcp"));
        assert!(!injection.bearer_token.is_empty());
        // Injection alone does not grant session approval; the UI does that
        // explicitly after session start.
        assert!(!rt.host().approvals().session_approved("s1"));
    }

    #[test]
    fn teardown_revokes_token_and_session_state() {
        let rt = runtime();
        let _injection = rt.prepare_injection("s1").expect("injection");
        rt.teardown_session("s1");
        assert!(!rt.host().approvals().session_approved("s1"));
        // Idempotent.
        rt.teardown_session("s1");
    }

    #[test]
    fn server_is_shared_across_sessions() {
        let rt = runtime();
        let a = rt.prepare_injection("s1").expect("a");
        let b = rt.prepare_injection("s2").expect("b");
        // Same server (same URL), different tokens.
        assert_eq!(a.url, b.url);
        assert_ne!(a.bearer_token, b.bearer_token);
    }
}
