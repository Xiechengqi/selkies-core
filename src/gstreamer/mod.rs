//! GStreamer integration for video encoding
//!
//! This module provides GStreamer-based video pipeline using appsrc
//! for encoding compositor frames to H.264/VP8/VP9 for WebRTC streaming.

pub mod pipeline;
pub mod encoder;

pub use pipeline::{VideoPipeline, PipelineConfig};


use std::error::Error;
use std::fmt;

/// GStreamer-related errors
#[derive(Debug)]
#[allow(dead_code)]
pub enum GstError {
    /// GStreamer initialization failed
    InitFailed(String),
    /// Pipeline creation failed
    PipelineFailed(String),
    /// Encoder not available
    EncoderNotFound(String),
    /// Element linking failed
    LinkFailed(String),
    /// State change failed
    StateChangeFailed(String),
    /// Feature not enabled
    FeatureDisabled,
}

impl fmt::Display for GstError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GstError::InitFailed(msg) => write!(f, "GStreamer init failed: {}", msg),
            GstError::PipelineFailed(msg) => write!(f, "Pipeline creation failed: {}", msg),
            GstError::EncoderNotFound(msg) => write!(f, "Encoder not found: {}", msg),
            GstError::LinkFailed(msg) => write!(f, "Element linking failed: {}", msg),
            GstError::StateChangeFailed(msg) => write!(f, "State change failed: {}", msg),
            GstError::FeatureDisabled => write!(f, "WebRTC streaming feature is not enabled"),
        }
    }
}

impl Error for GstError {}

/// Initialize GStreamer subsystem
#[allow(dead_code)]
pub fn init() -> Result<(), GstError> {
    gstreamer::init().map_err(|e| GstError::InitFailed(e.to_string()))
}

/// Check if GStreamer is available and properly initialized
#[allow(dead_code)]
pub fn is_available() -> bool {
    gstreamer::init().is_ok()
}
