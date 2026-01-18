//! JPEG encoding using turbojpeg
//!
//! Provides stripe-based JPEG encoding with change detection.

pub mod encoder;
pub use encoder::{Encoder, EncoderConfig, Stripe};
