//! Real platform providers for [`ComputerUseProvider`].
//!
//! macOS is the primary target (Phase 3): app/window enumeration via
//! `NSWorkspace` + the CGWindow list, screenshot via `screencapture`, AX
//! element-tree inspection via the Accessibility (`AXUIElement`) API, and input
//! injection via `CGEvent`. Other platforms get a stub that reports
//! `Unsupported`, keeping the host portable.
//!
//! ## Verification note
//!
//! The AX and `CGEvent` FFI here is net-new and the highest-risk part of the
//! project. It compiles and is structured against Apple's documented APIs, but
//! the live behaviour (element coordinates, synthesized-event delivery, Secure
//! Input interactions) must be verified on a real macOS desktop — it cannot be
//! exercised in a headless CI run. The host-level policy around it is covered by
//! the `FakeProvider` tests in `host.rs`.

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::MacosProvider;

#[cfg(not(target_os = "macos"))]
mod unsupported;

#[cfg(not(target_os = "macos"))]
pub use unsupported::UnsupportedProvider;

/// Construct the platform default provider for this build.
pub fn default_provider() -> std::sync::Arc<dyn super::provider::ComputerUseProvider> {
    #[cfg(target_os = "macos")]
    {
        std::sync::Arc::new(MacosProvider::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::sync::Arc::new(UnsupportedProvider::new())
    }
}
