//! selkies-core - Rust-based Selkies streaming core
//!
//! A high-performance streaming solution for X11 desktops with support for:
//! - WebRTC + GStreamer (default, low-latency)
//! - WebSocket + TurboJPEG (legacy, fallback)

pub mod config;
pub mod capture;
pub mod encode;
pub mod audio;
pub mod transport;
pub mod input;
pub mod web;

// WebRTC + GStreamer modules (feature-gated)
#[cfg(feature = "webrtc-streaming")]
pub mod gstreamer;

#[cfg(feature = "webrtc-streaming")]
pub mod webrtc;

// Re-exports
pub use config::{Config, WebRTCConfig, VideoCodec, HardwareEncoder};
pub use encode::{Encoder, Stripe};
pub use transport::WebSocketServer;
pub use input::InputInjector;

#[cfg(feature = "webrtc-streaming")]
pub use gstreamer::{VideoPipeline, PipelineConfig};

#[cfg(feature = "webrtc-streaming")]
pub use webrtc::{SessionManager, WebRTCSession, SignalingMessage};
