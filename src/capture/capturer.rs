//! Screen capture abstraction
//!
//! Provides a unified interface for screen capture with X11.

use crate::capture::frame::{Frame, FrameStats};
use std::sync::Arc;
use x11rb::errors::ConnectionError;
use x11rb::xcb_ffi::XCBConnection;

/// Trait for screen capture implementations
pub trait Capturer: Send {
    /// Capture a single frame
    fn capture(&mut self) -> Result<Frame, Box<dyn std::error::Error>>;

    /// Get capture statistics
    fn stats(&self) -> FrameStats;

    /// Check if capturer is still connected
    fn is_connected(&self) -> bool;
}

/// Create a capturer
pub fn create_capturer(
    conn: Arc<XCBConnection>,
    screen_num: i32,
) -> Result<super::X11Capturer, ConnectionError> {
    super::X11Capturer::new(conn, screen_num)
}
