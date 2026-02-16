//! Shared state for ivnc
//!
//! Manages shared configuration and WebRTC sessions.

#![allow(dead_code)]

use crate::config::Config;
use crate::config::ui::UiConfig;
use crate::audio::AudioPacket;
use xxhash_rust::xxh64::xxh64;
use crate::input::InputEventData;
use crate::runtime_settings::RuntimeSettings;
use base64::Engine;
use log::{info, warn};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::sync::mpsc;


/// Shared state for the application
#[derive(Clone)]
pub struct SharedState {
    /// Configuration
    pub config: Arc<Config>,

    /// RTP packet broadcast sender (for WebRTC mode)
    pub rtp_sender: broadcast::Sender<Vec<u8>>,

    /// Audio broadcast sender
    pub audio_sender: broadcast::Sender<AudioPacket>,

    /// Text broadcast sender (clipboard, stats, system messages)
    pub text_sender: broadcast::Sender<String>,

    /// Input event sender
    pub input_sender: mpsc::UnboundedSender<InputEventData>,

    /// Display dimensions
    pub display_size: Arc<Mutex<(u32, u32)>>,

    /// Clipboard content (base64 text)
    pub clipboard: Arc<Mutex<Option<String>>>,

    /// Request keyframe flag (for WebRTC)
    pub force_keyframe: Arc<AtomicBool>,

    /// Request pipeline rebuild (after display resize)
    pub pipeline_rebuild: Arc<AtomicBool>,

    /// Pending display resize target (width, height); pipeline thread will apply it
    pub pending_resize: Arc<Mutex<Option<(u32, u32)>>>,

    /// Runtime stats
    pub stats: Arc<Mutex<RuntimeStats>>,

    /// UI configuration
    pub ui_config: Arc<UiConfig>,

    /// Server start time
    pub start_time: std::time::Instant,

    /// Last cursor message payload
    pub last_cursor_message: Arc<Mutex<Option<String>>>,

    /// WebRTC session count
    pub webrtc_session_count: Arc<AtomicU64>,

    /// Bumped each time a DataChannel opens (used to trigger taskbar resend)
    pub datachannel_open_count: Arc<AtomicU64>,

    /// Runtime settings updated from client
    pub runtime_settings: Arc<RuntimeSettings>,

    /// Last received WebRTC stats (raw JSON)
    pub last_webrtc_stats_video: Arc<Mutex<Option<String>>>,
    pub last_webrtc_stats_audio: Arc<Mutex<Option<String>>>,

    /// Last clipboard hash written by server (to suppress echo)
    pub last_clipboard_write_hash: Arc<Mutex<Option<u64>>>,

    /// Flag: browser sent new clipboard content, compositor should pick it up
    pub clipboard_incoming_dirty: Arc<AtomicBool>,

    /// Channel for browser→compositor clipboard content (replaces dirty flag for new data)
    pub clipboard_incoming_tx: mpsc::UnboundedSender<String>,
    pub clipboard_incoming_rx: Arc<Mutex<mpsc::UnboundedReceiver<String>>>,

    /// Cached keyframe RTP packets for new session replay
    pub keyframe_cache: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl std::fmt::Debug for SharedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState")
            .field("config", &self.config)
            .field("display_size", &self.display_size)
            .field("webrtc_sessions", &self.webrtc_sessions())
            .finish()
    }
}

impl SharedState {
    /// Create a new shared state
    pub fn new(
        config: Config,
        ui_config: UiConfig,
        input_sender: mpsc::UnboundedSender<InputEventData>,
        runtime_settings: Arc<RuntimeSettings>,
    ) -> Self {
        let (rtp_sender, _) = broadcast::channel(2000);
        let (audio_sender, _) = broadcast::channel(500);
        let (text_sender, _) = broadcast::channel(256);
        let (clipboard_incoming_tx, clipboard_incoming_rx) = mpsc::unbounded_channel();
        let display_size = Arc::new(Mutex::new((config.display.width, config.display.height)));

        Self {
            config: Arc::new(config),
            ui_config: Arc::new(ui_config),
            rtp_sender,
            audio_sender,
            text_sender,
            input_sender,
            display_size,
            clipboard: Arc::new(Mutex::new(None)),
            force_keyframe: Arc::new(AtomicBool::new(false)),
            pipeline_rebuild: Arc::new(AtomicBool::new(false)),
            pending_resize: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(RuntimeStats::default())),
            start_time: std::time::Instant::now(),
            last_cursor_message: Arc::new(Mutex::new(None)),
            webrtc_session_count: Arc::new(AtomicU64::new(0)),
            datachannel_open_count: Arc::new(AtomicU64::new(0)),
            runtime_settings,
            last_webrtc_stats_video: Arc::new(Mutex::new(None)),
            last_webrtc_stats_audio: Arc::new(Mutex::new(None)),
            last_clipboard_write_hash: Arc::new(Mutex::new(None)),
            clipboard_incoming_dirty: Arc::new(AtomicBool::new(false)),
            clipboard_incoming_tx,
            clipboard_incoming_rx: Arc::new(Mutex::new(clipboard_incoming_rx)),
            keyframe_cache: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn update_webrtc_stats(&self, kind: &str, payload: &str) {
        match kind {
            "video" => {
                let mut last = self.last_webrtc_stats_video.lock().unwrap();
                *last = Some(payload.to_string());
            }
            "audio" => {
                let mut last = self.last_webrtc_stats_audio.lock().unwrap();
                *last = Some(payload.to_string());
            }
            _ => {}
        }
    }

    pub fn handle_command_message(&self, message: &str) -> bool {
        if !message.starts_with("cmd,") {
            return false;
        }
        if !self.config.input.enable_commands {
            warn!("Command execution disabled; ignoring cmd request");
            return true;
        }
        let cmd = message.trim_start_matches("cmd,").trim();
        if cmd.is_empty() {
            warn!("Received empty cmd request");
            return true;
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        match Command::new("sh")
            .arg("-lc")
            .arg(cmd)
            .current_dir(home)
            .spawn()
        {
            Ok(_) => info!("Launched command: {}", cmd),
            Err(err) => warn!("Failed to launch command '{}': {}", cmd, err),
        }
        true
    }

    pub fn handle_settings_message(&self, message: &str) -> bool {
        if !message.starts_with("SETTINGS,") {
            return false;
        }
        let payload = message.trim_start_matches("SETTINGS,");
        self.runtime_settings.apply_settings_json(payload);
        true
    }

    pub fn update_cursor_message(&self, message: String) {
        let mut last = self.last_cursor_message.lock().unwrap();
        *last = Some(message);
    }

    pub fn last_cursor_message(&self) -> Option<String> {
        self.last_cursor_message.lock().unwrap().clone()
    }

    /// Store clipboard and broadcast to clients
    pub fn set_clipboard(&self, base64_text: String) {
        let mut clipboard = self.clipboard.lock().unwrap();
        *clipboard = Some(base64_text.clone());
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&base64_text) {
            let total_size = decoded.len();
            if total_size > 8192 {
                let _ = self
                    .text_sender
                    .send(format!("clipboard_start,text/plain,{}", total_size));
                for chunk in decoded.chunks(4096) {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
                    let _ = self
                        .text_sender
                        .send(format!("clipboard_data,{}", encoded));
                }
                let _ = self.text_sender.send("clipboard_finish".to_string());
                return;
            }
        }

        let _ = self.text_sender.send(format!("clipboard,{}", base64_text));
    }

    /// Store binary clipboard and broadcast to clients
    pub fn set_clipboard_binary(&self, mime_type: String, data: Vec<u8>) {
        if data.len() > 8192 {
            let _ = self
                .text_sender
                .send(format!("clipboard_start,{},{}", mime_type, data.len()));
            for chunk in data.chunks(4096) {
                let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
                let _ = self
                    .text_sender
                    .send(format!("clipboard_data,{}", encoded));
            }
            let _ = self.text_sender.send("clipboard_finish".to_string());
            return;
        }

        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let _ = self
            .text_sender
            .send(format!("clipboard_binary,{},{}", mime_type, encoded));
    }

    pub fn mark_clipboard_written(&self, mime_type: &str, data: &[u8]) {
        let mut hash = xxh64(mime_type.as_bytes(), 0);
        hash = xxh64(data, hash);
        let mut last = self.last_clipboard_write_hash.lock().unwrap();
        *last = Some(hash);
    }

    pub fn last_clipboard_hash(&self) -> Option<u64> {
        *self.last_clipboard_write_hash.lock().unwrap()
    }

    /// Update display size
    pub fn set_display_size(&self, width: u32, height: u32) {
        let mut size = self.display_size.lock().unwrap();
        *size = (width, height);
    }

    /// Get current display size
    pub fn display_size(&self) -> (u32, u32) {
        *self.display_size.lock().unwrap()
    }

    /// Request display resize
    pub fn resize_display(&self, width: u32, height: u32) {
        let current = self.display_size();
        if current == (width, height) {
            return;
        }
        info!("Queuing display resize to {}x{}", width, height);
        *self.pending_resize.lock().unwrap() = Some((width, height));
    }

    /// Take pending resize request (called by compositor thread)
    pub fn take_pending_resize(&self) -> Option<(u32, u32)> {
        self.pending_resize.lock().unwrap().take()
    }

    /// Update resource usage stats
    pub fn update_resource_usage(&self, cpu_percent: f64, mem_used: u64) {
        let mut stats = self.stats.lock().unwrap();
        stats.cpu_percent = cpu_percent;
        stats.mem_used = mem_used;
    }

    /// Update latency metric (ms)
    pub fn update_latency(&self, latency_ms: u64) {
        let mut stats = self.stats.lock().unwrap();
        stats.latency_ms = latency_ms;
    }


    /// Update client-reported latency metric (ms)
    pub fn update_client_latency(&self, latency_ms: u64) {
        let mut stats = self.stats.lock().unwrap();
        stats.client_latency_ms = latency_ms;
    }

    /// Update client-reported FPS
    pub fn update_client_fps(&self, fps: u32) {
        let mut stats = self.stats.lock().unwrap();
        stats.client_fps = fps;
    }

    /// Record an ICE candidate (TCP-only keeps a minimal counter)
    pub fn record_ice_candidate(&self, transport: Option<&str>) {
        let mut stats = self.stats.lock().unwrap();
        stats.ice_candidates_total += 1;
        if transport == Some("tcp") {
            stats.ice_candidates_tcp += 1;
        }
    }

    /// Build stats JSON payload
    pub fn stats_json(&self) -> String {
        let stats = self.stats.lock().unwrap().clone();
        format!(
            r#"{{"fps":{:.2},"bandwidth":{},"latency":{},"client_latency":{},"client_fps":{},"clients":{},"cpu_percent":{:.1},"mem_used":{},"ice_candidates_total":{},"ice_candidates_tcp":{}}}"#,
            stats.fps,
            stats.bandwidth,
            stats.latency_ms,
            stats.client_latency_ms,
            stats.client_fps,
            self.connection_count(),
            stats.cpu_percent,
            stats.mem_used,
            stats.ice_candidates_total,
            stats.ice_candidates_tcp
        )
    }

    /// Build UI configuration JSON payload
    pub fn ui_config_json(&self) -> String {
        self.ui_config.to_json()
    }

    /// Get server uptime
    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Get connection count (WebRTC sessions)
    pub fn connection_count(&self) -> u64 {
        self.webrtc_sessions()
    }

    /// Shutdown the server
    pub async fn shutdown(&self) {
        info!("Shutting down shared state...");
    }

    // WebRTC methods

    /// Request a keyframe from the encoder
    pub fn request_keyframe(&self) {
        self.force_keyframe.store(true, Ordering::Relaxed);
    }

    /// Consume keyframe request flag
    pub fn take_keyframe_request(&self) -> bool {
        self.force_keyframe.swap(false, Ordering::Relaxed)
    }

    /// Consume pipeline rebuild flag
    pub fn take_pipeline_rebuild(&self) -> bool {
        self.pipeline_rebuild.swap(false, Ordering::Relaxed)
    }

    /// Broadcast an RTP packet to all WebRTC sessions
    pub fn broadcast_rtp(&self, packet: Vec<u8>) {
        let _ = self.rtp_sender.send(packet);
    }

    /// Update the keyframe cache with a new set of RTP packets
    pub fn set_keyframe_cache(&self, packets: Vec<Vec<u8>>) {
        if let Ok(mut cache) = self.keyframe_cache.lock() {
            *cache = packets;
        }
    }

    /// Get a clone of the cached keyframe packets
    pub fn get_keyframe_cache(&self) -> Vec<Vec<u8>> {
        self.keyframe_cache.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Subscribe to RTP packets
    pub fn subscribe_rtp(&self) -> broadcast::Receiver<Vec<u8>> {
        self.rtp_sender.subscribe()
    }

    /// Subscribe to text messages (cursor, clipboard, stats)
    pub fn subscribe_text(&self) -> broadcast::Receiver<String> {
        self.text_sender.subscribe()
    }

    /// Subscribe to audio packets
    pub fn subscribe_audio(&self) -> broadcast::Receiver<AudioPacket> {
        self.audio_sender.subscribe()
    }

    /// Increment WebRTC session count
    pub fn increment_webrtc_sessions(&self) {
        self.webrtc_session_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement WebRTC session count (saturating to avoid underflow)
    pub fn decrement_webrtc_sessions(&self) {
        let mut current = self.webrtc_session_count.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                break;
            }
            match self.webrtc_session_count.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// Get WebRTC session count
    pub fn webrtc_sessions(&self) -> u64 {
        self.webrtc_session_count.load(Ordering::Relaxed)
    }

    /// Get video codec from WebRTC config
    pub fn video_codec(&self) -> crate::config::VideoCodec {
        self.config.webrtc.video_codec
    }

    /// Build extended stats JSON payload including WebRTC info
    #[allow(dead_code)]
    pub fn extended_stats_json(&self) -> String {
        let stats = self.stats.lock().unwrap().clone();
        let webrtc_sessions = self.webrtc_sessions();

        format!(
            r#"{{"fps":{:.2},"bandwidth":{},"latency":{},"client_latency":{},"client_fps":{},"clients":{},"cpu_percent":{:.1},"mem_used":{},"webrtc_sessions":{},"ice_candidates_total":{},"ice_candidates_tcp":{}}}"#,
            stats.fps,
            stats.bandwidth,
            stats.latency_ms,
            stats.client_latency_ms,
            stats.client_fps,
            self.connection_count(),
            stats.cpu_percent,
            stats.mem_used,
            webrtc_sessions,
            stats.ice_candidates_total,
            stats.ice_candidates_tcp
        )
    }
}

/// Runtime stats snapshot
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    pub fps: f64,
    pub bandwidth: u64,
    pub latency_ms: u64,
    pub client_latency_ms: u64,
    pub client_fps: u32,
    pub total_frames: u64,
    pub total_bytes: u64,
    pub cpu_percent: f64,
    pub mem_used: u64,
    pub ice_candidates_total: u64,
    pub ice_candidates_tcp: u64,
}

impl Default for RuntimeStats {
    fn default() -> Self {
        Self {
            fps: 0.0,
            bandwidth: 0,
            latency_ms: 0,
            client_latency_ms: 0,
            client_fps: 0,
            total_frames: 0,
            total_bytes: 0,
            cpu_percent: 0.0,
            mem_used: 0,
            ice_candidates_total: 0,
            ice_candidates_tcp: 0,
        }
    }
}
