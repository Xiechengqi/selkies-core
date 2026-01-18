use clap::Parser;
use std::path::PathBuf;

use crate::config;

#[derive(Parser, Debug)]
#[command(name = "selkies-core")]
#[command(author = "Selkies Team")]
#[command(version = "0.1.0")]
#[command(about = "Rust-based Selkies streaming core", long_about = None)]
pub struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/selkies-core.toml")]
    pub config: PathBuf,

    /// Display width
    #[arg(long, default_value = "1920")]
    pub width: u32,

    /// Display height
    #[arg(long, default_value = "1080")]
    pub height: u32,

    /// Display number (X11 display)
    #[arg(short, long, default_value = ":0")]
    pub display: String,

    /// WebSocket port
    #[arg(short, long)]
    pub port: Option<u16>,

    /// HTTP port for health/UI
    #[arg(long)]
    pub http_port: Option<u16>,

    /// Verbose logging
    #[arg(short, long, action)]
    pub verbose: bool,

    /// Run in foreground (don't daemonize)
    #[arg(long, action)]
    pub foreground: bool,

    /// PID file path
    #[arg(long, default_value = "/var/run/selkies-core.pid")]
    pub pidfile: PathBuf,

    /// Disable automatic X11 display management
    #[arg(long, action)]
    pub no_auto_x11: bool,

    /// Force X11 backend (auto, xvfb, xdummy, or none)
    #[arg(long)]
    pub x11_backend: Option<String>,

    /// X11 display number range for auto-allocation (e.g., "99-199")
    #[arg(long)]
    pub x11_display_range: Option<String>,

    /// Timeout for X11 startup in seconds
    #[arg(long)]
    pub x11_startup_timeout: Option<u64>,
}

impl Args {
    pub fn load_config(&self) -> Result<config::Config, Box<dyn std::error::Error>> {
        config::Config::load(&self.config)
    }
}
