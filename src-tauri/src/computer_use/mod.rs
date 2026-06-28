//! Host-owned `computer use` module.
//!
//! This is the agent-agnostic core of the computer-use feature: lease
//! lifecycle, snapshot staleness, provider abstraction, permission gating, and
//! session/app approvals. It is deliberately independent of *how* the tool is
//! injected into an agent (ACP MCP server vs Pi extension) so it can be built
//! and tested independently from the injection plumbing.
//!
//! Privileged OS operations live behind the [`provider::ComputerUseProvider`]
//! trait. The real macOS provider handles screen capture, AX element inspection,
//! and `CGEvent` input injection; this module is exercised in tests with a
//! deterministic fake provider.

pub mod approvals;
pub mod broker;
pub mod config;
pub mod host;
pub mod lease;
pub mod mcp_http;
pub mod onboarding;
pub mod permissions;
pub mod pi_extension;
pub mod platform;
pub mod provider;
pub mod settings;
pub mod skill_resource;

pub use approvals::{AppApproval, ApprovalDecision, ApprovalRegistry, SessionApproval};
pub use host::{ComputerUseError, ComputerUseHost};
pub use lease::{Lease, LeaseRegistry, SnapshotId};
pub use mcp_http::{McpHttpServer, McpServerHandle};
pub use onboarding::{
    ComputerUsePermissions, GrantPermissionResult, PermissionKind, PermissionRequirement,
};
pub use platform::default_provider;
pub use provider::{
    AllowedAction, AppLaunchResult, AppListOptions, AppState, AppTarget, ComputerUseProvider,
    CoordinateSpace, DisplayMetadata, ElementId, InstalledApp, Point, ScreenshotRef, UiElement,
};
pub use settings::ComputerUseSettings;
