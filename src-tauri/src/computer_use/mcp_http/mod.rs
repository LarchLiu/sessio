//! Desktop-owned, loopback-only HTTP MCP server for `computer use`.
//!
//! This is the v3 ACP injection shape: the desktop process hosts a localhost
//! HTTP MCP server, injects it into `session/new` as `McpServer::Http`, and
//! handles all `computer_*` tool calls itself — keeping privileged OS work in
//! Sessio while talking to a separate ACP agent child process.
//!
//! The module is split so the security-critical parts are pure and unit-testable
//! without binding a socket:
//!
//! - [`auth`] — per-session bearer-token registry + loopback/scope checks.
//! - [`protocol`] — MCP JSON-RPC framing (`initialize` / `tools/list` /
//!   `tools/call`) and the `computer_*` tool schema.
//! - [`dispatch`] — maps a validated tool call to the [`ComputerUseHost`].
//! - [`server`] — the `tiny_http` lifecycle (ephemeral loopback port, shutdown).
//!
//! Security model (see plan v3 "HTTP MCP Security Model"): bind 127.0.0.1 only,
//! ephemeral port, per-session random bearer token, reject anything that does
//! not map to a live `computerUse` session, and expire tokens on session end.

pub mod auth;
pub mod dispatch;
pub mod protocol;
pub mod server;

pub use auth::{AuthError, SessionToken, TokenRegistry};
pub use protocol::{McpRequest, McpResponse, TOOL_DEFINITIONS};
pub use server::{McpHttpServer, McpServerHandle};
