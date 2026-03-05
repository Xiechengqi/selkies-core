//! Pake application management module
//!
//! Manages web-to-desktop applications with native (Chrome --app) and webview modes.

pub mod app;
pub mod store;
pub mod process;
pub mod native;
pub mod webview;
pub mod webview_manager_process;
pub mod datadir;
pub mod autostart;
pub mod api;
pub mod state_recovery;

// Re-export the process-based WebViewManager as the default
pub use webview_manager_process::WebViewManager;
