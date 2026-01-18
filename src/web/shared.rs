//! Shared state for selkies-core
//!
//! Manages client connections, shared configuration, and WebRTC sessions.

#![allow(dead_code)]

use crate::config::Config;
use crate::config::ui::UiConfig;
use crate::audio::AudioPacket;
use crate::encode::Stripe;
use crate::input::{InputEvent, InputEventData};
use base64::Engine;
use log::{debug, info};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use uuid::Uuid;


/// Client connection information
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// Unique client ID
    pub id: String,

    /// Client address
    pub address: SocketAddr,

    /// Connected at
    pub connected_at: std::time::Instant,
}

impl ClientInfo {
    /// Create a new client info
    pub fn new(address: SocketAddr) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            address,
            connected_at: std::time::Instant::now(),
        }
    }
}

/// Streaming mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingMode {
    /// WebRTC with GStreamer (low-latency)
    WebRTC,
    /// Legacy WebSocket with JPEG stripes
    WebSocket,
    /// Hybrid mode (WebRTC preferred, WebSocket fallback)
    Hybrid,
}

impl Default for StreamingMode {
    fn default() -> Self {
        #[cfg(feature = "webrtc-streaming")]
        return StreamingMode::Hybrid;

        #[cfg(not(feature = "webrtc-streaming"))]
        return StreamingMode::WebSocket;
    }
}

/// Shared state for the application
#[derive(Clone)]
pub struct SharedState {
    /// Configuration
    pub config: Arc<Config>,

    /// Connected clients (WebSocket)
    pub clients: Arc<Mutex<HashMap<String, ClientInfo>>>,

    /// Frame broadcast sender (for WebSocket/JPEG mode)
    pub frame_sender: broadcast::Sender<Vec<Stripe>>,

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

    /// Force full refresh flag
    pub force_refresh: Arc<AtomicBool>,

    /// Request keyframe flag (for WebRTC)
    pub force_keyframe: Arc<AtomicBool>,

    /// Runtime stats
    pub stats: Arc<Mutex<RuntimeStats>>,

    /// UI configuration
    pub ui_config: Arc<UiConfig>,

    /// Server start time
    pub start_time: std::time::Instant,

    /// Total connections
    pub total_connections: Arc<Mutex<u64>>,

    /// Last cursor message payload
    pub last_cursor_message: Arc<Mutex<Option<String>>>,

    /// Per-client mouse button mask state
    pub client_button_masks: Arc<Mutex<HashMap<String, u8>>>,

    /// Per-client last mouse position
    pub client_mouse_positions: Arc<Mutex<HashMap<String, (i32, i32)>>>,

    /// Current streaming mode
    pub streaming_mode: Arc<Mutex<StreamingMode>>,

    /// WebRTC session count
    pub webrtc_session_count: Arc<AtomicU64>,

    /// WebSocket client count
    pub websocket_client_count: Arc<AtomicU64>,
}

impl std::fmt::Debug for SharedState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedState")
            .field("config", &self.config)
            .field("clients_count", &self.clients.lock().map(|c| c.len()).unwrap_or(0))
            .field("display_size", &self.display_size)
            .field("streaming_mode", &self.streaming_mode)
            .finish()
    }
}

impl SharedState {
    /// Create a new shared state
    pub fn new(
        config: Config,
        ui_config: UiConfig,
        input_sender: mpsc::UnboundedSender<InputEventData>,
    ) -> Self {
        let (frame_sender, _) = broadcast::channel(100);
        let (rtp_sender, _) = broadcast::channel(500);  // More capacity for RTP packets
        let (audio_sender, _) = broadcast::channel(100);
        let (text_sender, _) = broadcast::channel(100);
        let display_size = Arc::new(Mutex::new((config.display.width, config.display.height)));

        // Determine initial streaming mode
        let streaming_mode = if config.webrtc.enabled {
            StreamingMode::Hybrid
        } else {
            StreamingMode::WebSocket
        };

        Self {
            config: Arc::new(config),
            ui_config: Arc::new(ui_config),
            clients: Arc::new(Mutex::new(HashMap::new())),
            frame_sender,
            rtp_sender,
            audio_sender,
            text_sender,
            input_sender,
            display_size,
            clipboard: Arc::new(Mutex::new(None)),
            force_refresh: Arc::new(AtomicBool::new(false)),
            force_keyframe: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(RuntimeStats::default())),
            start_time: std::time::Instant::now(),
            total_connections: Arc::new(Mutex::new(0)),
            client_button_masks: Arc::new(Mutex::new(HashMap::new())),
            client_mouse_positions: Arc::new(Mutex::new(HashMap::new())),
            last_cursor_message: Arc::new(Mutex::new(None)),
            streaming_mode: Arc::new(Mutex::new(streaming_mode)),
            webrtc_session_count: Arc::new(AtomicU64::new(0)),
            websocket_client_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Add a new client
    pub fn add_client(&self, address: SocketAddr) -> String {
        let mut clients = self.clients.lock().unwrap();
        let mut total = self.total_connections.lock().unwrap();
        *total += 1;

        let client = ClientInfo::new(address);
        let id = client.id.clone();
        clients.insert(id.clone(), client);
        self.client_button_masks.lock().unwrap().insert(id.clone(), 0);
        self.client_mouse_positions.lock().unwrap().insert(id.clone(), (0, 0));

        info!("Client connected: {} (total: {})", id, *total);
        id
    }

    /// Remove a client
    pub fn remove_client(&self, client_id: &str) {
        let mut clients = self.clients.lock().unwrap();
        clients.remove(client_id);
        self.client_button_masks.lock().unwrap().remove(client_id);
        self.client_mouse_positions.lock().unwrap().remove(client_id);
        info!("Client disconnected: {}", client_id);
    }

    /// Get all clients
    pub fn get_all_clients(&self) -> Vec<ClientInfo> {
        let clients = self.clients.lock().unwrap();
        clients.values().cloned().collect()
    }

    /// Inject mouse move
    pub fn inject_mouse_move(&self, _client_id: &str, x: i32, y: i32) {
        debug!("Mouse move: ({}, {})", x, y);
        let mut event = InputEventData::default();
        event.event_type = InputEvent::MouseMove;
        event.mouse_x = x;
        event.mouse_y = y;
        let _ = self.input_sender.send(event);
    }

    /// Inject mouse button
    pub fn inject_mouse_button(&self, _client_id: &str, button: u8, pressed: bool) {
        debug!("Mouse button: {} = {}", button, pressed);
        let mut event = InputEventData::default();
        event.event_type = InputEvent::MouseButton;
        event.mouse_button = button;
        event.button_pressed = pressed;
        let _ = self.input_sender.send(event);
    }

    /// Inject mouse wheel
    pub fn inject_mouse_wheel(&self, _client_id: &str, dx: i16, dy: i16) {
        debug!("Mouse wheel: ({}, {})", dx, dy);
        let mut event = InputEventData::default();
        event.event_type = InputEvent::MouseWheel;
        event.wheel_delta_x = dx;
        event.wheel_delta_y = dy;
        let _ = self.input_sender.send(event);
    }

    /// Inject keyboard
    pub fn inject_keyboard(&self, _client_id: &str, keysym: u32, pressed: bool) {
        debug!("Keyboard: keysym=0x{:x} pressed={}", keysym, pressed);
        let mut event = InputEventData::default();
        event.event_type = InputEvent::Keyboard;
        event.keysym = keysym;
        event.key_pressed = pressed;
        let _ = self.input_sender.send(event);
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

    /// Request a full refresh from the capture loop
    pub fn request_full_refresh(&self) {
        self.force_refresh.store(true, Ordering::Relaxed);
    }

    /// Consume refresh request flag
    pub fn take_refresh_request(&self) -> bool {
        self.force_refresh.swap(false, Ordering::Relaxed)
    }

    /// Update display size from capture backend
    pub fn set_display_size(&self, width: u32, height: u32) {
        let mut size = self.display_size.lock().unwrap();
        *size = (width, height);
    }

    /// Get current display size
    pub fn display_size(&self) -> (u32, u32) {
        *self.display_size.lock().unwrap()
    }

    /// Update capture stats
    pub fn update_capture_stats(&self, bytes: usize, fps: f64) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_frames += 1;
        stats.total_bytes += bytes as u64;
        stats.fps = fps;
        stats.bandwidth = (bytes as f64 * fps) as u64;
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

    /// Build stats JSON payload
    pub fn stats_json(&self) -> String {
        let stats = self.stats.lock().unwrap().clone();
        format!(
            r#"{{"fps":{:.2},"bandwidth":{},"latency":{},"clients":{},"cpu_percent":{:.1},"mem_used":{}}}"#,
            stats.fps,
            stats.bandwidth,
            stats.latency_ms,
            self.connection_count(),
            stats.cpu_percent,
            stats.mem_used
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

    /// Get connection count
    pub fn connection_count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }

    /// Shutdown the server
    pub async fn shutdown(&self) {
        info!("Shutting down shared state...");
    }

    // WebRTC-specific methods

    /// Get current streaming mode
    pub fn get_streaming_mode(&self) -> StreamingMode {
        *self.streaming_mode.lock().unwrap()
    }

    /// Set streaming mode
    pub fn set_streaming_mode(&self, mode: StreamingMode) {
        let mut current = self.streaming_mode.lock().unwrap();
        if *current != mode {
            info!("Streaming mode changed: {:?} -> {:?}", *current, mode);
            *current = mode;
        }
    }

    /// Request a keyframe from the encoder
    pub fn request_keyframe(&self) {
        self.force_keyframe.store(true, Ordering::Relaxed);
    }

    /// Consume keyframe request flag
    pub fn take_keyframe_request(&self) -> bool {
        self.force_keyframe.swap(false, Ordering::Relaxed)
    }

    /// Broadcast an RTP packet to all WebRTC sessions
    pub fn broadcast_rtp(&self, packet: Vec<u8>) {
        let _ = self.rtp_sender.send(packet);
    }

    /// Subscribe to RTP packets
    pub fn subscribe_rtp(&self) -> broadcast::Receiver<Vec<u8>> {
        self.rtp_sender.subscribe()
    }

    /// Increment WebRTC session count
    pub fn increment_webrtc_sessions(&self) {
        self.webrtc_session_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement WebRTC session count
    pub fn decrement_webrtc_sessions(&self) {
        self.webrtc_session_count.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get WebRTC session count
    pub fn webrtc_sessions(&self) -> u64 {
        self.webrtc_session_count.load(Ordering::Relaxed)
    }

    /// Increment WebSocket client count
    pub fn increment_websocket_clients(&self) {
        self.websocket_client_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement WebSocket client count
    pub fn decrement_websocket_clients(&self) {
        self.websocket_client_count.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get WebSocket client count
    #[allow(dead_code)]
    pub fn websocket_clients(&self) -> u64 {
        self.websocket_client_count.load(Ordering::Relaxed)
    }

    /// Check if WebRTC is enabled in configuration
    #[allow(dead_code)]
    pub fn is_webrtc_enabled(&self) -> bool {
        self.config.webrtc.enabled
    }

    /// Get video codec from WebRTC config
    #[cfg(feature = "webrtc-streaming")]
    pub fn video_codec(&self) -> crate::config::VideoCodec {
        self.config.webrtc.video_codec
    }

    /// Build extended stats JSON payload including WebRTC info
    #[allow(dead_code)]
    pub fn extended_stats_json(&self) -> String {
        let stats = self.stats.lock().unwrap().clone();
        let mode = self.get_streaming_mode();
        let webrtc_sessions = self.webrtc_sessions();
        let ws_clients = self.websocket_clients();

        format!(
            r#"{{"fps":{:.2},"bandwidth":{},"latency":{},"clients":{},"cpu_percent":{:.1},"mem_used":{},"streaming_mode":"{:?}","webrtc_sessions":{},"ws_clients":{}}}"#,
            stats.fps,
            stats.bandwidth,
            stats.latency_ms,
            self.connection_count(),
            stats.cpu_percent,
            stats.mem_used,
            mode,
            webrtc_sessions,
            ws_clients
        )
    }
}

/// Runtime stats snapshot
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    pub fps: f64,
    pub bandwidth: u64,
    pub latency_ms: u64,
    pub total_frames: u64,
    pub total_bytes: u64,
    pub cpu_percent: f64,
    pub mem_used: u64,
}

impl Default for RuntimeStats {
    fn default() -> Self {
        Self {
            fps: 0.0,
            bandwidth: 0,
            latency_ms: 0,
            total_frames: 0,
            total_bytes: 0,
            cpu_percent: 0.0,
            mem_used: 0,
        }
    }
}
