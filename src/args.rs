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

    /// Enable or disable Trickle ICE (true/false)
    #[arg(long)]
    pub webrtc_ice_trickle: Option<bool>,

    /// NAT 1:1 external IP mappings (comma-separated list)
    #[arg(long)]
    pub webrtc_nat1to1: Option<String>,

    /// URL used to fetch external IP when NAT mappings are not provided
    #[arg(long)]
    pub webrtc_ip_retrieval_url: Option<String>,

    /// Network profile for WebRTC ("lan" or "wan")
    #[arg(long)]
    pub webrtc_profile: Option<String>,

    /// STUN host (optional)
    #[arg(long)]
    pub webrtc_stun_host: Option<String>,

    /// STUN port (optional)
    #[arg(long)]
    pub webrtc_stun_port: Option<u16>,

    /// TURN host (optional)
    #[arg(long)]
    pub webrtc_turn_host: Option<String>,

    /// TURN port (optional)
    #[arg(long)]
    pub webrtc_turn_port: Option<u16>,

    /// TURN protocol: "udp" or "tcp"
    #[arg(long)]
    pub webrtc_turn_protocol: Option<String>,

    /// Enable TURN over TLS/DTLS
    #[arg(long)]
    pub webrtc_turn_tls: Option<bool>,

    /// TURN shared secret for HMAC credentials
    #[arg(long)]
    pub webrtc_turn_shared_secret: Option<String>,

    /// TURN username for legacy auth
    #[arg(long)]
    pub webrtc_turn_username: Option<String>,

    /// TURN password for legacy auth
    #[arg(long)]
    pub webrtc_turn_password: Option<String>,

    /// Ephemeral UDP port range for ICE (e.g., "59000-59100")
    #[arg(long)]
    pub webrtc_ephemeral_udp_port_range: Option<String>,

    /// Single UDP mux port for all peers
    #[arg(long)]
    pub webrtc_udp_mux_port: Option<u16>,

    /// Single TCP mux port for all peers
    #[arg(long)]
    pub webrtc_tcp_mux_port: Option<u16>,

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
