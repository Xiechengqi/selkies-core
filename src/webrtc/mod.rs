//! WebRTC streaming implementation
//!
//! This module provides WebRTC-based video streaming with:
//! - Peer connection management
//! - SDP/ICE signaling
//! - RTP video transmission
//! - DataChannel for input events

#[cfg(feature = "webrtc-streaming")]
pub mod peer_connection;

#[cfg(feature = "webrtc-streaming")]
pub mod signaling;

#[cfg(feature = "webrtc-streaming")]
pub mod data_channel;

#[cfg(feature = "webrtc-streaming")]
pub mod media_track;

#[cfg(feature = "webrtc-streaming")]
pub mod session;


#[cfg(feature = "webrtc-streaming")]
pub use signaling::SignalingMessage;


#[cfg(feature = "webrtc-streaming")]
pub use session::SessionManager;

#[cfg(feature = "webrtc-streaming")]
#[allow(unused_imports)]
pub use session::WebRTCSession;

use std::error::Error;
use std::fmt;

/// WebRTC-related errors
#[derive(Debug)]
#[allow(dead_code)]
pub enum WebRTCError {
    /// Peer connection creation failed
    ConnectionFailed(String),
    /// SDP processing failed
    SdpError(String),
    /// ICE candidate processing failed
    IceError(String),
    /// Data channel error
    DataChannelError(String),
    /// Media track error
    MediaError(String),
    /// Session not found
    SessionNotFound(String),
    /// Invalid state transition
    InvalidState(String),
    /// Feature not enabled
    FeatureDisabled,
}

impl fmt::Display for WebRTCError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebRTCError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            WebRTCError::SdpError(msg) => write!(f, "SDP error: {}", msg),
            WebRTCError::IceError(msg) => write!(f, "ICE error: {}", msg),
            WebRTCError::DataChannelError(msg) => write!(f, "DataChannel error: {}", msg),
            WebRTCError::MediaError(msg) => write!(f, "Media error: {}", msg),
            WebRTCError::SessionNotFound(id) => write!(f, "Session not found: {}", id),
            WebRTCError::InvalidState(msg) => write!(f, "Invalid state: {}", msg),
            WebRTCError::FeatureDisabled => write!(f, "WebRTC streaming feature is not enabled"),
        }
    }
}

impl Error for WebRTCError {}

/// Check if WebRTC feature is available
#[allow(dead_code)]
pub fn is_available() -> bool {
    cfg!(feature = "webrtc-streaming")
}
