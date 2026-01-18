//! Transport layer for Selkies streaming
//!
//! Handles WebSocket connections for legacy streaming and WebRTC signaling.

pub mod websocket;
pub mod codec;

#[cfg(feature = "webrtc-streaming")]
pub mod signaling_server;

pub use websocket::WebSocketServer;

#[cfg(feature = "webrtc-streaming")]
pub use signaling_server::handle_signaling_connection;
