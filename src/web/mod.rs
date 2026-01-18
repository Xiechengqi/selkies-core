//! HTTP server for health checks and metrics
//!
//! Provides a lightweight HTTP server for monitoring.

pub mod shared;
pub use shared::SharedState;

pub mod http_server;
pub use http_server::run_http_server_with_webrtc;
