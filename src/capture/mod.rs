//! X11 Screen capture
//!
//! Provides screen capture using X11 XImage.

mod xshm;
pub use xshm::X11Capturer;

pub mod frame;
pub use frame::Frame;
