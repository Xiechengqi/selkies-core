// Display number allocation and conflict detection

use super::detector::{DisplayDetector, DisplayStatus};
use super::{DisplayError, Result};
use log::{debug, trace};
use std::path::Path;

/// Display number allocator
pub struct DisplayAllocator;

impl DisplayAllocator {
    /// Find an available display number in the given range
    pub fn find_available_display(start: u32, end: u32) -> Result<u32> {
        debug!("Searching for available display in range {}..{}", start, end);

        for num in start..=end {
            if !Self::is_display_in_use(num) {
                debug!("Found available display number: {}", num);
                return Ok(num);
            }
        }

        Err(DisplayError::NoAvailableDisplay)
    }

    /// Check if a display number is currently in use
    fn is_display_in_use(num: u32) -> bool {
        // Method 1: Check if socket file exists
        let socket_path = format!("/tmp/.X11-unix/X{}", num);
        if Path::new(&socket_path).exists() {
            trace!("Display :{} socket file exists", num);
            return true;
        }

        // Method 2: Try to connect to the display
        Self::can_connect_to_display(num)
    }

    /// Attempt to connect to a display to verify it's in use
    fn can_connect_to_display(num: u32) -> bool {
        let display = format!(":{}", num);
        matches!(DisplayDetector::check_display(&display), DisplayStatus::Available)
    }
}
