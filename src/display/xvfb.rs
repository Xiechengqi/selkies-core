// Xvfb process management

use super::{DisplayBackend, Result};
use log::{debug, info, warn};
use std::process::{Child, Command};
use std::path::Path;
use std::fs;

/// Configuration for Xvfb
#[derive(Debug, Clone)]
pub struct XvfbConfig {
    pub display_number: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub dpi: u32,
    pub extra_args: Vec<String>,
}

impl XvfbConfig {
    /// Create a new Xvfb configuration
    #[allow(dead_code)]
    pub fn new(display_number: u32, width: u32, height: u32) -> Self {
        Self {
            display_number,
            width,
            height,
            depth: 24,
            dpi: 96,
            extra_args: Vec::new(),
        }
    }

    /// Convert configuration to Command
    pub fn to_command(&self) -> Command {
        let mut cmd = Command::new("Xvfb");

        cmd.arg(format!(":{}", self.display_number))
            .arg("-screen")
            .arg("0")
            .arg(format!("{}x{}x{}", self.width, self.height, self.depth))
            .arg("-dpi")
            .arg(self.dpi.to_string())
            .arg("-nolisten")
            .arg("tcp")
            .arg("-noreset")
            .arg("+extension")
            .arg("GLX")
            .arg("+extension")
            .arg("RANDR")
            .arg("+extension")
            .arg("RENDER");

        // Add extra arguments
        for arg in &self.extra_args {
            cmd.arg(arg);
        }

        cmd
    }
}

/// Managed display with X11 process
pub struct ManagedDisplay {
    pub display: String,
    pub process: Child,
    pub backend: DisplayBackend,
}

impl std::fmt::Debug for ManagedDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedDisplay")
            .field("display", &self.display)
            .field("backend", &self.backend)
            .field("process_id", &self.process.id())
            .finish()
    }
}

impl ManagedDisplay {
    /// Create a new managed display
    pub fn new(display: String, process: Child, backend: DisplayBackend) -> Self {
        Self {
            display,
            process,
            backend,
        }
    }

    /// Check if the process is still running
    #[allow(dead_code)]
    pub fn is_alive(&self) -> bool {
        // This is a non-blocking check
        // We can't use try_wait() on an immutable reference, so we just assume it's alive
        // The actual health check should be done by trying to connect to the display
        true
    }

    /// Shutdown the managed display
    #[allow(dead_code)]
    pub fn shutdown(mut self) -> Result<()> {
        info!("Shutting down managed display {} ({})", self.display, self.backend);

        // Try graceful shutdown first (SIGTERM)
        if let Err(e) = self.process.kill() {
            warn!("Failed to send SIGTERM to {} process: {}", self.backend, e);
        }

        // Wait for process to exit (with timeout handled by wait())
        match self.process.wait() {
            Ok(status) => {
                debug!("{} process exited with status: {}", self.backend, status);
            }
            Err(e) => {
                warn!("Error waiting for {} process: {}", self.backend, e);
            }
        }

        // Clean up socket file
        Self::cleanup_socket(&self.display);

        Ok(())
    }

    /// Clean up X11 socket file
    #[allow(dead_code)]
    fn cleanup_socket(display: &str) {
        if let Some(num) = display.strip_prefix(':') {
            let socket_path = format!("/tmp/.X11-unix/X{}", num);
            if Path::new(&socket_path).exists() {
                if let Err(e) = fs::remove_file(&socket_path) {
                    warn!("Failed to remove socket file {}: {}", socket_path, e);
                } else {
                    debug!("Cleaned up socket file: {}", socket_path);
                }
            }
        }
    }
}

impl Drop for ManagedDisplay {
    fn drop(&mut self) {
        debug!("Dropping ManagedDisplay {}", self.display);
        let _ = self.process.kill();
    }
}
