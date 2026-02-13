//! Transport layer for iVnc streaming
//!
//! Handles WebRTC signaling over WebSocket.

pub mod signaling_server;

pub use signaling_server::handle_signaling_connection;
