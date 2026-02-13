//! Configuration management for ivnc

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod ui;

/// Video codec selection for WebRTC streaming
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    #[default]
    H264,
    VP8,
    VP9,
    AV1,
}

impl VideoCodec {
    pub fn as_str(&self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::VP8 => "vp8",
            VideoCodec::VP9 => "vp9",
            VideoCodec::AV1 => "av1",
        }
    }

    #[allow(dead_code)]
    pub fn mime_type(&self) -> &'static str {
        match self {
            VideoCodec::H264 => "video/H264",
            VideoCodec::VP8 => "video/VP8",
            VideoCodec::VP9 => "video/VP9",
            VideoCodec::AV1 => "video/AV1",
        }
    }

    #[allow(dead_code)]
    pub fn rtp_payload_type(&self) -> u8 {
        match self {
            VideoCodec::H264 => 96,
            VideoCodec::VP8 => 97,
            VideoCodec::VP9 => 98,
            VideoCodec::AV1 => 99,
        }
    }
}

/// Hardware encoder selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HardwareEncoder {
    #[default]
    Auto,
    Software,
    Vaapi,   // Intel VA-API
    Nvenc,   // NVIDIA NVENC
    Qsv,     // Intel Quick Sync
}

impl HardwareEncoder {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            HardwareEncoder::Auto => "auto",
            HardwareEncoder::Software => "software",
            HardwareEncoder::Vaapi => "vaapi",
            HardwareEncoder::Nvenc => "nvenc",
            HardwareEncoder::Qsv => "qsv",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Server configuration
    pub server: ServerConfig,

    /// Display configuration
    pub display: DisplayConfig,

    /// HTTP configuration
    pub http: HttpConfig,

    /// Encoding configuration
    pub encoding: EncodingConfig,

    /// Input configuration
    pub input: InputConfig,

    /// Audio configuration
    #[serde(default)]
    pub audio: AudioConfig,

    /// Logging configuration
    pub logging: LoggingConfig,

    /// WebRTC configuration
    #[serde(default)]
    pub webrtc: WebRTCConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Run in foreground
    pub foreground: bool,

    /// PID file path
    pub pidfile: PathBuf,

    /// User to run as (for privilege dropping)
    pub user: Option<String>,

    /// Group to run as
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// Screen width in pixels
    pub width: u32,

    /// Screen height in pixels
    pub height: u32,

    /// Refresh rate in Hz
    pub refresh_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// HTTP bind address
    pub host: String,

    /// HTTP port for health checks
    pub port: u16,

    /// CORS origin
    pub cors_origin: Option<String>,

    /// Enable HTTP basic authentication
    #[serde(default = "default_basic_auth_enabled")]
    pub basic_auth_enabled: bool,

    /// Basic auth username
    #[serde(default = "default_basic_auth_user")]
    pub basic_auth_user: String,

    /// Basic auth password
    #[serde(default = "default_basic_auth_password")]
    pub basic_auth_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingConfig {
    /// Target FPS
    pub target_fps: u32,

    /// Maximum FPS
    pub max_fps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    /// Enable keyboard input
    pub enable_keyboard: bool,

    /// Enable mouse input
    pub enable_mouse: bool,

    /// Enable clipboard sync
    pub enable_clipboard: bool,

    /// Enable binary clipboard sync
    #[serde(default)]
    pub enable_binary_clipboard: bool,

    /// Enable command execution from client messages
    #[serde(default)]
    pub enable_commands: bool,


    /// Allowed file transfer directions ("upload", "download")
    #[serde(default = "default_file_transfers")]
    pub file_transfers: Vec<String>,

    /// Directory to store uploaded files
    #[serde(default = "default_upload_dir")]
    pub upload_dir: String,

    /// Mouse sensitivity multiplier
    pub mouse_sensitivity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AudioConfig {
    /// Enable audio streaming
    pub enabled: bool,

    /// Sample rate (Hz)
    pub sample_rate: u32,

    /// Channel count
    pub channels: u16,

    /// Bitrate (bps)
    pub bitrate: u32,
}

/// ICE server configuration for WebRTC NAT traversal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceServerConfig {
    /// STUN/TURN server URLs (e.g., "stun:stun.l.google.com:19302")
    pub urls: Vec<String>,

    /// Username for TURN authentication (optional)
    #[serde(default)]
    pub username: Option<String>,

    /// Credential for TURN authentication (optional)
    #[serde(default)]
    pub credential: Option<String>,
}

impl Default for IceServerConfig {
    fn default() -> Self {
        Self {
            urls: vec!["stun:stun.l.google.com:19302".to_string()],
            username: None,
            credential: None,
        }
    }
}

/// WebRTC streaming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRTCConfig {
    /// Enable WebRTC streaming (if false, falls back to WebSocket mode)
    pub enabled: bool,

    /// Optional network profile ("lan" or "wan")
    #[serde(default)]
    pub network_profile: Option<String>,

    /// Enable Trickle ICE (send candidates as they are found)
    #[serde(default = "default_ice_trickle")]
    pub ice_trickle: bool,

    /// ICE servers for NAT traversal
    #[serde(default)]
    pub ice_servers: Vec<IceServerConfig>,

    /// STUN host for NAT traversal (optional)
    #[serde(default)]
    pub stun_host: String,

    /// STUN port for NAT traversal (optional)
    #[serde(default)]
    pub stun_port: u16,

    /// TURN host for relay (optional)
    #[serde(default)]
    pub turn_host: String,

    /// TURN port for relay (optional)
    #[serde(default)]
    pub turn_port: u16,

    /// TURN transport protocol: "udp" or "tcp"
    #[serde(default = "default_turn_protocol")]
    pub turn_protocol: String,

    /// Enable TURN over TLS/DTLS
    #[serde(default)]
    pub turn_tls: bool,

    /// TURN shared secret for HMAC credentials (optional)
    #[serde(default)]
    pub turn_shared_secret: String,

    /// TURN username for legacy auth (optional)
    #[serde(default)]
    pub turn_username: String,

    /// TURN password for legacy auth (optional)
    #[serde(default)]
    pub turn_password: String,

    /// Video codec selection
    #[serde(default)]
    pub video_codec: VideoCodec,

    /// Target video bitrate in kbps
    pub video_bitrate: u32,

    /// Maximum video bitrate in kbps
    pub video_bitrate_max: u32,

    /// Minimum video bitrate in kbps
    pub video_bitrate_min: u32,

    /// Hardware encoder preference
    #[serde(default)]
    pub hardware_encoder: HardwareEncoder,

    /// Enable adaptive bitrate control
    pub adaptive_bitrate: bool,

    /// Congestion control algorithm ("goog-remb", "transport-cc")
    pub congestion_control: String,

    /// Maximum latency target in milliseconds
    pub max_latency_ms: u32,

    /// Enable forward error correction
    pub fec_enabled: bool,

    /// Enable retransmissions (RTX)
    pub rtx_enabled: bool,

    /// GStreamer pipeline latency in ms
    pub pipeline_latency_ms: u32,

    /// Keyframe interval in frames
    pub keyframe_interval: u32,

    /// NAT 1:1 external IP mappings (optional)
    #[serde(default)]
    pub nat1to1_ips: Vec<String>,

    /// URL used to fetch external IP when NAT mappings are not provided
    #[serde(default)]
    pub ip_retrieval_url: String,

    /// Ephemeral UDP port range for ICE (optional)
    #[serde(default)]
    pub ephemeral_udp_port_range: Option<[u16; 2]>,

    /// Single UDP mux port for all peers (0 to disable)
    #[serde(default)]
    pub udp_mux_port: u16,

    /// Single TCP mux port for all peers (0 to disable)
    #[serde(default)]
    pub tcp_mux_port: u16,
}

impl Default for WebRTCConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            network_profile: None,
            ice_trickle: true,
            ice_servers: vec![IceServerConfig::default()],
            stun_host: "stun.l.google.com".to_string(),
            stun_port: 19302,
            turn_host: String::new(),
            turn_port: 3478,
            turn_protocol: "udp".to_string(),
            turn_tls: false,
            turn_shared_secret: String::new(),
            turn_username: String::new(),
            turn_password: String::new(),
            video_codec: VideoCodec::H264,
            video_bitrate: 8000,       // 8 Mbps default (screen content needs higher bitrate)
            video_bitrate_max: 16000,  // 16 Mbps max
            video_bitrate_min: 1000,   // 1 Mbps min
            hardware_encoder: HardwareEncoder::Auto,
            adaptive_bitrate: true,
            congestion_control: "goog-remb".to_string(),
            max_latency_ms: 100,
            fec_enabled: false,
            rtx_enabled: true,
            pipeline_latency_ms: 50,
            keyframe_interval: 60,
            nat1to1_ips: Vec::new(),
            ip_retrieval_url: String::new(),
            ephemeral_udp_port_range: None,
            udp_mux_port: 0,
            tcp_mux_port: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level
    pub level: String,

    /// Log file path
    pub logfile: Option<PathBuf>,

    /// Log format
    pub format: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                foreground: false,
                pidfile: PathBuf::from("/var/run/ivnc.pid"),
                user: None,
                group: None,
            },
            display: DisplayConfig {
                width: 1920,
                height: 1080,
                refresh_rate: 60,
            },
            http: HttpConfig {
                host: "0.0.0.0".to_string(),
                port: 8008,
                cors_origin: None,
                basic_auth_enabled: true,
                basic_auth_user: "user".to_string(),
                basic_auth_password: "mypasswd".to_string(),
            },
            encoding: EncodingConfig {
                target_fps: 30,
                max_fps: 60,
            },
            input: InputConfig {
                enable_keyboard: true,
                enable_mouse: true,
                enable_clipboard: true,
                enable_binary_clipboard: false,
                enable_commands: false,
                file_transfers: default_file_transfers(),
                upload_dir: default_upload_dir(),
                mouse_sensitivity: 1.0,
            },
            audio: AudioConfig {
                enabled: false,
                sample_rate: 48_000,
                channels: 2,
                bitrate: 128_000,
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                logfile: None,
                format: "json".to_string(),
            },
            webrtc: WebRTCConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from TOML file
    pub fn load(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.exists() {
            return Ok(Config::default());
        }

        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.display.width == 0 || self.display.height == 0 {
            return Err("Display dimensions must be non-zero".into());
        }

        if self.encoding.target_fps > self.encoding.max_fps {
            return Err("Target FPS cannot exceed max FPS".into());
        }

        if self.http.basic_auth_enabled && self.http.basic_auth_password.is_empty() {
            return Err("Basic auth is enabled but password is empty".into());
        }

        for entry in &self.input.file_transfers {
            let value = entry.trim().to_ascii_lowercase();
            if value.is_empty() || value == "none" {
                continue;
            }
            if value != "upload" && value != "download" {
                return Err("Input file_transfers must contain \"upload\" or \"download\"".into());
            }
        }

        if let Some(range) = self.webrtc.ephemeral_udp_port_range {
            if range[0] == 0 || range[1] == 0 || range[0] > range[1] {
                return Err("Invalid WebRTC ephemeral UDP port range".into());
            }
        }

        if !self.webrtc.turn_protocol.is_empty()
            && self.webrtc.turn_protocol != "udp"
            && self.webrtc.turn_protocol != "tcp"
        {
            return Err("WebRTC TURN protocol must be \"udp\" or \"tcp\"".into());
        }

        if (!self.webrtc.turn_shared_secret.is_empty()
            || !self.webrtc.turn_username.is_empty()
            || !self.webrtc.turn_password.is_empty())
            && self.webrtc.turn_host.is_empty()
        {
            return Err("WebRTC TURN host is required when TURN auth is configured".into());
        }

        if self.audio.enabled {
            if self.audio.sample_rate == 0 {
                return Err("Audio sample rate must be non-zero".into());
            }
            if self.audio.channels == 0 || self.audio.channels > 2 {
                return Err("Audio channels must be 1 or 2".into());
            }
            if self.audio.bitrate == 0 {
                return Err("Audio bitrate must be non-zero".into());
            }
        }

        // WebRTC validation
        if self.webrtc.enabled {
            if self.webrtc.video_bitrate == 0 {
                return Err("WebRTC video bitrate must be non-zero".into());
            }
            if self.webrtc.video_bitrate_min > self.webrtc.video_bitrate {
                return Err("WebRTC min bitrate cannot exceed target bitrate".into());
            }
            if self.webrtc.video_bitrate > self.webrtc.video_bitrate_max {
                return Err("WebRTC target bitrate cannot exceed max bitrate".into());
            }
            if self.webrtc.keyframe_interval == 0 {
                return Err("WebRTC keyframe interval must be non-zero".into());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn validate_rejects_invalid_dimensions() {
        let mut cfg = Config::default();
        cfg.display.width = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_audio_requires_channels() {
        let mut cfg = Config::default();
        cfg.audio.enabled = true;
        cfg.audio.channels = 3;
        assert!(cfg.validate().is_err());
    }
}

fn default_basic_auth_enabled() -> bool {
    true
}

fn default_basic_auth_user() -> String {
    "user".to_string()
}

fn default_basic_auth_password() -> String {
    "mypasswd".to_string()
}

fn default_file_transfers() -> Vec<String> {
    vec!["upload".to_string(), "download".to_string()]
}

fn default_upload_dir() -> String {
    "~/Desktop".to_string()
}

fn default_ice_trickle() -> bool {
    true
}

fn default_turn_protocol() -> String {
    "udp".to_string()
}
