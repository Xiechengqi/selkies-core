// Display detection and availability checking

use super::Result;
use log::{debug, trace};
use std::ffi::CString;
use std::time::{Duration, Instant};
use std::thread;
use x11rb::xcb_ffi::XCBConnection;

/// Display availability status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayStatus {
    /// Display is available and can be connected to
    Available,
    /// Display is unavailable with error message
    Unavailable(String),
}

impl DisplayStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, DisplayStatus::Available)
    }
}

/// Display detector for checking X11 display availability
pub struct DisplayDetector;

impl DisplayDetector {
    /// Check if a display is available and can be connected to
    pub fn check_display(display: &str) -> DisplayStatus {
        trace!("Checking display availability: {}", display);

        // Try to create a CString for the display
        let display_cstr = match CString::new(display) {
            Ok(s) => s,
            Err(e) => {
                return DisplayStatus::Unavailable(format!("Invalid display string: {}", e));
            }
        };

        // Attempt to connect to the display
        match XCBConnection::connect(Some(display_cstr.as_c_str())) {
            Ok((conn, _screen_num)) => {
                debug!("Successfully connected to display {}", display);
                drop(conn);
                DisplayStatus::Available
            }
            Err(e) => {
                trace!("Failed to connect to display {}: {}", display, e);
                DisplayStatus::Unavailable(e.to_string())
            }
        }
    }

    /// Wait for a display to become available (used after starting Xvfb)
    pub fn wait_for_display(display: &str, timeout: Duration) -> Result<()> {
        debug!("Waiting for display {} to become ready (timeout: {:?})", display, timeout);

        let start = Instant::now();

        while start.elapsed() < timeout {
            match Self::check_display(display) {
                DisplayStatus::Available => {
                    debug!("Display {} is ready", display);
                    return Ok(());
                }
                DisplayStatus::Unavailable(_) => {
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }

        Err(super::DisplayError::DisplayTimeout)
    }
}
