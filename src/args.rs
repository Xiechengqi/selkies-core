use clap::Parser;
use std::path::PathBuf;

use crate::config;

#[derive(Parser, Debug)]
#[command(name = "ivnc")]
#[command(author = "iVnc Team")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "iVnc streaming core", long_about = None)]
pub struct Args {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/ivnc.toml")]
    pub config: PathBuf,

    /// Display width
    #[arg(long, default_value = "1920")]
    pub width: u32,

    /// Display height
    #[arg(long, default_value = "1080")]
    pub height: u32,

    /// HTTP port for health/UI
    #[arg(long)]
    pub http_port: Option<u16>,

    /// Enable or disable basic authentication (true/false)
    #[arg(long)]
    pub basic_auth_enabled: Option<bool>,

    /// Basic authentication username
    #[arg(long)]
    pub basic_auth_user: Option<String>,

    /// Basic authentication password
    #[arg(long)]
    pub basic_auth_password: Option<String>,

    /// Enable or disable binary clipboard (true/false)
    #[arg(long)]
    pub binary_clipboard_enabled: Option<bool>,

    /// Enable or disable client command execution (true/false)
    #[arg(long)]
    pub commands_enabled: Option<bool>,

    /// Allowed file transfer directions (comma-separated: "upload,download")
    #[arg(long)]
    pub file_transfers: Option<String>,

    /// Directory to store uploaded files
    #[arg(long)]
    pub upload_dir: Option<String>,

    /// Public candidate address for ICE-TCP (e.g., "1.2.3.4:8008")
    #[arg(long)]
    pub webrtc_public_candidate: Option<String>,

    /// Allow candidate override from Host header (true/false)
    #[arg(long)]
    pub webrtc_candidate_from_host_header: Option<bool>,

    /// Verbose logging
    #[arg(short, long, action)]
    pub verbose: bool,

    /// Run in foreground (don't daemonize)
    #[arg(long, action)]
    pub foreground: bool,

    /// Enable HTTPS with auto-generated self-signed certificate
    #[arg(long, action)]
    pub tls: bool,

    /// PID file path
    #[arg(long, default_value = "/var/run/ivnc.pid")]
    pub pidfile: PathBuf,
}

impl Args {
    pub fn load_config(&self) -> Result<config::Config, Box<dyn std::error::Error>> {
        config::Config::load(&self.config)
    }
}
