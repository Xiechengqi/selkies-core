//! selkies-core - Rust-based Selkies streaming core
//!
//! A high-performance WebRTC streaming solution for X11 desktops using GStreamer.

pub mod config;
pub mod audio;
pub mod clipboard;
pub mod file_upload;
pub mod runtime_settings;
pub mod system_clipboard;
pub mod transport;
pub mod input;
pub mod web;
pub mod display;
pub mod gstreamer;
pub mod webrtc;

// Re-exports
pub use config::{Config, WebRTCConfig, VideoCodec, HardwareEncoder};
pub use input::InputInjector;
pub use gstreamer::{VideoPipeline, PipelineConfig};
pub use webrtc::{SessionManager, WebRTCSession, SignalingMessage};
