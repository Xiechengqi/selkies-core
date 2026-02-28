//! iVnc - WebRTC streaming core
//!
//! A high-performance WebRTC streaming solution using smithay Wayland compositor and GStreamer.

pub mod config;
pub mod audio;
pub mod clipboard;
pub mod system_clipboard;
pub mod file_upload;
pub mod runtime_settings;
pub mod transport;
pub mod input;
pub mod web;
pub mod compositor;
pub mod gstreamer;
pub mod webrtc;
#[cfg(feature = "mcp")]
pub mod mcp;

// Re-exports
pub use config::{Config, WebRTCConfig, VideoCodec, HardwareEncoder};
pub use input::{InputEvent, InputEventData};
pub use gstreamer::{VideoPipeline, PipelineConfig};
pub use webrtc::{SessionManager, SignalingMessage};
