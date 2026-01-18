//! X11 Capture Frame data structure
//!
//! Represents a captured screen frame with metadata.

use std::fmt;

/// Represents a captured frame from X11
#[derive(Debug, Clone)]
pub struct Frame {
    /// Frame width in pixels
    pub width: u32,

    /// Frame height in pixels
    pub height: u32,

    /// Raw pixel data (RGB format)
    pub data: Vec<u8>,

    /// Capture timestamp
    pub timestamp: std::time::Instant,

    /// Frame sequence number
    pub sequence: u64,

    /// Dirty region indicator (true if frame has changes)
    pub is_dirty: bool,
}

impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Frame({}x{}, {} bytes, seq={})",
            self.width,
            self.height,
            self.data.len(),
            self.sequence
        )
    }
}

/// Frame statistics for monitoring
#[derive(Debug, Default, Clone)]
pub struct FrameStats {
    /// Total frames captured
    pub total_frames: u64,

    /// Total bytes captured
    pub total_bytes: u64,

    /// Total capture time in microseconds
    pub total_capture_time_us: u64,

    /// Last capture time in microseconds
    pub last_capture_time_us: u64,
}

impl FrameStats {
    /// Record a frame capture
    pub fn record_capture(&mut self, bytes: usize, time_us: u64) {
        self.total_frames += 1;
        self.total_bytes += bytes as u64;
        self.last_capture_time_us = time_us;
        self.total_capture_time_us += time_us;
    }
}
