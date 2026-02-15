//! WebRTC streaming implementation (str0m Sans-I/O)
//!
//! This module provides WebRTC-based video streaming with:
//! - str0m Sans-I/O peer connection management
//! - Same-port HTTP + ICE-TCP protocol multiplexing
//! - RTP video/audio transmission
//! - DataChannel for input events

pub mod data_channel;
pub mod media_track;
pub mod rtc_session;
pub mod session;
pub mod signaling;
pub mod tcp_framing;

pub use session::SessionManager;
pub use signaling::SignalingMessage;

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
