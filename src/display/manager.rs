// Display manager - core logic for automatic X11 display management

use super::allocator::DisplayAllocator;
use super::detector::{DisplayDetector, DisplayStatus};
use super::xvfb::{ManagedDisplay, XvfbConfig};
use super::{DisplayBackend, DisplayError, Result};
use log::{debug, info, warn};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Display source - either existing or managed
#[derive(Debug)]
pub enum DisplaySource {
    /// Using an existing display (from environment or config)
    Existing(String),
    /// Using a managed display (auto-created)
    Managed(ManagedDisplay),
}

/// Configuration for display management
#[derive(Debug, Clone)]
pub struct DisplayManagerConfig {
    pub preferred_display: String,
    pub auto_x11: bool,
    pub x11_backend: String,
    pub x11_display_range: [u32; 2],
    pub x11_startup_timeout: u64,
    pub x11_extra_args: Vec<String>,
    pub width: u32,
    pub height: u32,
}

/// Display manager
pub struct DisplayManager {
    source: DisplaySource,
}

impl DisplayManager {
    /// Create a new display manager with automatic detection and creation
    pub fn new(config: DisplayManagerConfig) -> Result<Self> {
        info!("Initializing display manager");

        // Strategy 1: Try to use existing display
        if let Some(display) = Self::try_existing_display(&config) {
            info!("Using existing display: {}", display);
            return Ok(Self {
                source: DisplaySource::Existing(display),
            });
        }

        // Strategy 2: Try to create managed display if auto_x11 is enabled
        if config.auto_x11 {
            match Self::try_create_managed_display(&config) {
                Ok(managed) => {
                    info!("Created managed display: {} ({})", managed.display, managed.backend);
                    return Ok(Self {
                        source: DisplaySource::Managed(managed),
                    });
                }
                Err(e) => {
                    warn!("Failed to create managed display: {}", e);
                }
            }
        }

        // Strategy 3: No display available
        Err(DisplayError::NoDisplayAvailable)
    }

    /// Get the display string
    pub fn display(&self) -> &str {
        match &self.source {
            DisplaySource::Existing(display) => display,
            DisplaySource::Managed(managed) => &managed.display,
        }
    }

    /// Check if the display is still alive
    #[allow(dead_code)]
    pub fn is_alive(&self) -> bool {
        match DisplayDetector::check_display(self.display()) {
            DisplayStatus::Available => true,
            DisplayStatus::Unavailable(_) => false,
        }
    }

    /// Shutdown the display manager
    #[allow(dead_code)]
    pub fn shutdown(self) -> Result<()> {
        match self.source {
            DisplaySource::Existing(_) => {
                debug!("Using existing display, no cleanup needed");
                Ok(())
            }
            DisplaySource::Managed(managed) => managed.shutdown(),
        }
    }

    /// Try to use an existing display
    fn try_existing_display(config: &DisplayManagerConfig) -> Option<String> {
        // 1. Check environment variable
        if let Ok(env_display) = std::env::var("DISPLAY") {
            if !env_display.is_empty() {
                debug!("Checking DISPLAY from environment: {}", env_display);
                if DisplayDetector::check_display(&env_display).is_available() {
                    return Some(env_display);
                }
            }
        }

        // 2. Check configured display
        debug!("Checking configured display: {}", config.preferred_display);
        if DisplayDetector::check_display(&config.preferred_display).is_available() {
            return Some(config.preferred_display.clone());
        }

        None
    }

    /// Try to create a managed display
    fn try_create_managed_display(config: &DisplayManagerConfig) -> Result<ManagedDisplay> {
        // 1. Select backend
        let backend = Self::select_backend(&config.x11_backend)?;
        info!("Selected X11 backend: {}", backend);

        // 2. Allocate display number
        let display_num = DisplayAllocator::find_available_display(
            config.x11_display_range[0],
            config.x11_display_range[1],
        )?;
        info!("Allocated display number: :{}", display_num);

        // 3. Start X11 server
        let process = Self::start_xvfb(display_num, config)?;
        let display = format!(":{}", display_num);

        // 4. Wait for display to be ready
        DisplayDetector::wait_for_display(
            &display,
            Duration::from_secs(config.x11_startup_timeout),
        )?;

        Ok(ManagedDisplay::new(display, process, backend))
    }

    /// Select X11 backend
    fn select_backend(backend_str: &str) -> Result<DisplayBackend> {
        match backend_str {
            "xvfb" => Ok(DisplayBackend::Xvfb),
            "xdummy" => Ok(DisplayBackend::Xdummy),
            "auto" => {
                // Prefer Xvfb (more common)
                if Self::is_command_available("Xvfb") {
                    Ok(DisplayBackend::Xvfb)
                } else {
                    Err(DisplayError::NoX11BackendAvailable)
                }
            }
            _ => Err(DisplayError::InvalidBackend(backend_str.to_string())),
        }
    }

    /// Start Xvfb process
    fn start_xvfb(display_num: u32, config: &DisplayManagerConfig) -> Result<std::process::Child> {
        let xvfb_config = XvfbConfig {
            display_number: display_num,
            width: config.width,
            height: config.height,
            depth: 24,
            dpi: 96,
            extra_args: config.x11_extra_args.clone(),
        };

        let mut cmd = xvfb_config.to_command();
        cmd.stdout(Stdio::null()).stderr(Stdio::null());

        debug!("Starting Xvfb with command: {:?}", cmd);

        cmd.spawn()
            .map_err(|e| DisplayError::ProcessError(format!("Failed to spawn Xvfb: {}", e)))
    }

    /// Check if a command is available in PATH
    fn is_command_available(cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
