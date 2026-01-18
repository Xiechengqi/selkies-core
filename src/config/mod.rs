//! Configuration management for selkies-core

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

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

    /// WebSocket configuration
    pub websocket: WebSocketConfig,

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

    /// WebRTC configuration (optional, defaults to enabled)
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
    /// X11 display number (e.g., ":0")
    pub display: String,

    /// Screen width in pixels
    pub width: u32,

    /// Screen height in pixels
    pub height: u32,

    /// Refresh rate in Hz
    pub refresh_rate: u32,

    /// Xinerama/RANDR enable
    pub multi_monitor: bool,

    /// Enable automatic X11 display management
    #[serde(default = "default_auto_x11")]
    pub auto_x11: bool,

    /// X11 backend selection: "auto", "xvfb", "xdummy", or "none"
    #[serde(default = "default_x11_backend")]
    pub x11_backend: String,

    /// Display number range for auto-allocation [start, end]
    #[serde(default = "default_x11_display_range")]
    pub x11_display_range: [u32; 2],

    /// X11 startup timeout in seconds
    #[serde(default = "default_x11_startup_timeout")]
    pub x11_startup_timeout: u64,

    /// Extra arguments to pass to Xvfb
    #[serde(default)]
    pub x11_extra_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    /// Bind host
    pub host: String,

    /// WebSocket port
    pub port: u16,

    /// Maximum connections
    pub max_connections: u32,

    /// Connection timeout
    pub connection_timeout: Duration,

    /// Enable per-message deflate
    pub compression: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// HTTP bind address
    pub host: String,

    /// HTTP port for health checks
    pub port: u16,

    /// CORS origin
    pub cors_origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingConfig {
    /// JPEG quality (1-100)
    pub jpeg_quality: u8,

    /// Stripe height in pixels (must be power of 2)
    pub stripe_height: u32,

    /// Target FPS
    pub target_fps: u32,

    /// Maximum FPS
    pub max_fps: u32,

    /// Keyframe interval (in frames)
    pub keyframe_interval: u32,

    /// Bandwidth limit in bytes/sec (0 = unlimited)
    pub bandwidth_limit: u64,

    /// Enable adaptive quality
    pub adaptive_quality: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    /// Enable keyboard input
    pub enable_keyboard: bool,

    /// Enable mouse input
    pub enable_mouse: bool,

    /// Enable clipboard sync
    pub enable_clipboard: bool,

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

    /// ICE servers for NAT traversal
    #[serde(default)]
    pub ice_servers: Vec<IceServerConfig>,

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
}

impl Default for WebRTCConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ice_servers: vec![IceServerConfig::default()],
            video_codec: VideoCodec::H264,
            video_bitrate: 4000,       // 4 Mbps default
            video_bitrate_max: 8000,   // 8 Mbps max
            video_bitrate_min: 500,    // 500 kbps min
            hardware_encoder: HardwareEncoder::Auto,
            adaptive_bitrate: true,
            congestion_control: "goog-remb".to_string(),
            max_latency_ms: 100,
            fec_enabled: false,
            rtx_enabled: true,
            pipeline_latency_ms: 50,
            keyframe_interval: 60,
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
                pidfile: PathBuf::from("/var/run/selkies-core.pid"),
                user: None,
                group: None,
            },
            display: DisplayConfig {
                display: ":0".to_string(),
                width: 1920,
                height: 1080,
                refresh_rate: 60,
                multi_monitor: false,
                auto_x11: true,
                x11_backend: "auto".to_string(),
                x11_display_range: [99, 199],
                x11_startup_timeout: 10,
                x11_extra_args: Vec::new(),
            },
            websocket: WebSocketConfig {
                host: "0.0.0.0".to_string(),
                port: 8007,
                max_connections: 100,
                connection_timeout: Duration::from_secs(30),
                compression: true,
            },
            http: HttpConfig {
                host: "0.0.0.0".to_string(),
                port: 8008,
                cors_origin: None,
            },
            encoding: EncodingConfig {
                jpeg_quality: 75,
                stripe_height: 64,
                target_fps: 30,
                max_fps: 60,
                keyframe_interval: 300,
                bandwidth_limit: 0,
                adaptive_quality: true,
            },
            input: InputConfig {
                enable_keyboard: true,
                enable_mouse: true,
                enable_clipboard: true,
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

        if self.encoding.jpeg_quality > 100 || self.encoding.jpeg_quality < 1 {
            return Err("JPEG quality must be between 1 and 100".into());
        }

        if self.encoding.stripe_height == 0 {
            return Err("Stripe height must be non-zero".into());
        }

        if self.encoding.target_fps > self.encoding.max_fps {
            return Err("Target FPS cannot exceed max FPS".into());
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
    fn validate_rejects_invalid_quality() {
        let mut cfg = Config::default();
        cfg.encoding.jpeg_quality = 0;
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

// Default value functions for DisplayConfig
fn default_auto_x11() -> bool {
    true
}

fn default_x11_backend() -> String {
    "auto".to_string()
}

fn default_x11_display_range() -> [u32; 2] {
    [99, 199]
}

fn default_x11_startup_timeout() -> u64 {
    10
}
