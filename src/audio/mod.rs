//! Audio capture and encoding pipeline.

#[cfg(feature = "audio")]
mod runtime;

#[cfg(not(feature = "audio"))]
mod runtime;

pub use runtime::{run_audio_capture, AudioConfig, AudioPacket};
