// Display management module for automatic X11 display allocation
//
// This module provides automatic X11 display detection and management,
// including spawning virtual X servers (Xvfb) when no display is available.

mod allocator;
mod detector;
mod manager;
mod xvfb;

pub use manager::{DisplayManager, DisplayManagerConfig};

use std::fmt;

/// Display backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayBackend {
    /// X Virtual Framebuffer
    Xvfb,
    /// Xorg with dummy driver
    Xdummy,
}

impl fmt::Display for DisplayBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisplayBackend::Xvfb => write!(f, "Xvfb"),
            DisplayBackend::Xdummy => write!(f, "Xdummy"),
        }
    }
}

/// Display management errors
#[derive(Debug)]
#[allow(dead_code)]
pub enum DisplayError {
    /// No display available and auto-creation disabled or failed
    NoDisplayAvailable,
    /// No available display number in the configured range
    NoAvailableDisplay,
    /// No X11 backend (Xvfb/Xdummy) available on the system
    NoX11BackendAvailable,
    /// Display startup timeout
    DisplayTimeout,
    /// X11 connection error
    ConnectionError(String),
    /// Process spawn error
    ProcessError(String),
    /// Invalid backend specified
    InvalidBackend(String),
    /// Permission denied
    PermissionDenied(String),
}

impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisplayError::NoDisplayAvailable => {
                write!(f, "No X11 display available. Please set DISPLAY environment variable or enable auto_x11 in config.")
            }
            DisplayError::NoAvailableDisplay => {
                write!(f, "No available display number in the configured range. All displays are in use.")
            }
            DisplayError::NoX11BackendAvailable => {
                write!(f, "No X11 backend available. Please install Xvfb: apt-get install xvfb")
            }
            DisplayError::DisplayTimeout => {
                write!(f, "Timeout waiting for X11 display to become ready")
            }
            DisplayError::ConnectionError(msg) => {
                write!(f, "X11 connection error: {}", msg)
            }
            DisplayError::ProcessError(msg) => {
                write!(f, "Process error: {}", msg)
            }
            DisplayError::InvalidBackend(backend) => {
                write!(f, "Invalid X11 backend: {}. Valid options: auto, xvfb, xdummy", backend)
            }
            DisplayError::PermissionDenied(msg) => {
                write!(f, "Permission denied: {}", msg)
            }
        }
    }
}

impl std::error::Error for DisplayError {}

pub type Result<T> = std::result::Result<T, DisplayError>;
