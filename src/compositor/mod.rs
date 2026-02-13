//! Wayland compositor module based on smithay
//!
//! Provides an embedded headless Wayland compositor using smithay's
//! Pixman software renderer for zero-copy frame capture.

pub mod state;
pub mod headless;
pub mod handlers;
pub mod grabs;

pub use state::Compositor;
pub use headless::HeadlessBackend;
