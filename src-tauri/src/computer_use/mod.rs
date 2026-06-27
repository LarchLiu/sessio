//! Host-owned `computer use` module.
//!
//! This is the agent-agnostic core of the computer-use feature: lease
//! lifecycle, snapshot staleness, provider abstraction, permission gating, and
//! session/app approvals. It is deliberately independent of *how* the tool is
//! injected into an agent (ACP MCP server vs Pi extension) so it can be built
//! and tested before the injection plumbing (Phase 4) exists.
//!
//! Privileged OS operations live behind the [`provider::ComputerUseProvider`]
//! trait. The real macOS provider (screen capture via the CGWindow path already
//! in `lib.rs`, AX element inspection, and `CGEvent` input injection) is the
//! net-new, highest-risk work of Phase 3; this module is exercised in tests with
//! a deterministic fake provider.

pub mod approvals;
pub mod config;
pub mod host;
pub mod lease;
pub mod mcp_http;
pub mod permissions;
pub mod platform;
pub mod provider;
pub mod settings;

pub use approvals::{AppApproval, ApprovalDecision, ApprovalRegistry, SessionApproval};
pub use host::{ComputerUseError, ComputerUseHost};
pub use lease::{Lease, LeaseRegistry, SnapshotId};
pub use mcp_http::{McpHttpServer, McpServerHandle};
pub use platform::default_provider;
pub use provider::{
    AllowedAction, AppState, AppTarget, ComputerUseProvider, DisplayMetadata, ElementId,
    InstalledApp, UiElement,
};
pub use settings::ComputerUseSettings;
