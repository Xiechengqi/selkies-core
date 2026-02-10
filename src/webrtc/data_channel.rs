//! WebRTC DataChannel for input handling
//!
//! Handles bidirectional input events over WebRTC DataChannel,
//! compatible with the existing WebSocket input protocol.

#![allow(dead_code)]

use super::WebRTCError;
use crate::input::{InputEvent, InputEventData};
use crate::file_upload::FileUploadHandler;
use crate::runtime_settings::RuntimeSettings;
use crate::clipboard::ClipboardReceiver;
use crate::web::SharedState;
use log::{info, warn, debug, error};
use std::sync::Arc;
use tokio::sync::mpsc;
use std::sync::Mutex;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;

/// Input DataChannel handler
pub struct InputDataChannel {
    channel: Arc<RTCDataChannel>,
    input_tx: mpsc::UnboundedSender<InputEventData>,
    upload_handler: Arc<Mutex<FileUploadHandler>>,
    clipboard: Arc<Mutex<ClipboardReceiver>>,
    runtime_settings: Arc<RuntimeSettings>,
    shared_state: Arc<SharedState>,
}

impl InputDataChannel {
    /// Create a new input data channel handler
    pub fn new(
        channel: Arc<RTCDataChannel>,
        input_tx: mpsc::UnboundedSender<InputEventData>,
        upload_handler: Arc<Mutex<FileUploadHandler>>,
        clipboard: Arc<Mutex<ClipboardReceiver>>,
        runtime_settings: Arc<RuntimeSettings>,
        shared_state: Arc<SharedState>,
    ) -> Self {
        Self {
            channel,
            input_tx,
            upload_handler,
            clipboard,
            runtime_settings,
            shared_state,
        }
    }

    /// Set up message handling callbacks
    pub async fn setup_handlers(&self) {
        let input_tx = self.input_tx.clone();
        let upload_handler = self.upload_handler.clone();
        let clipboard = self.clipboard.clone();
        let runtime_settings = self.runtime_settings.clone();
        let shared_state = self.shared_state.clone();
        let channel_label = self.channel.label().to_string();

        // Handle incoming messages
        self.channel.on_message(Box::new(move |msg: DataChannelMessage| {
            let input_tx = input_tx.clone();
            let label = channel_label.clone();
            let upload_handler = upload_handler.clone();
            let clipboard = clipboard.clone();
            let runtime_settings = runtime_settings.clone();
            let shared_state = shared_state.clone();

            Box::pin(async move {
                if msg.is_string {
                    let text = match std::str::from_utf8(&msg.data) {
                        Ok(text) => text,
                        Err(e) => {
                            debug!("Failed to parse UTF-8 from {}: {}", label, e);
                            return;
                        }
                    };
                    if upload_handler.lock().unwrap_or_else(|e| e.into_inner()).handle_control_message(text) {
                        return;
                    }
                    if clipboard.lock().unwrap_or_else(|e| e.into_inner()).handle_message(text) {
                        return;
                    }
                    if shared_state.handle_command_message(text) {
                        return;
                    }
                    if text.starts_with("SETTINGS,") {
                        let payload = text.trim_start_matches("SETTINGS,");
                        runtime_settings.apply_settings_json(payload);
                        return;
                    }
                    if runtime_settings.handle_simple_message(text) {
                        return;
                    }
                    if text == "kr" {
                        return;
                    }
                    if text.starts_with("s,") {
                        return;
                    }
                    if text.starts_with("r,") {
                        let payload = text.trim_start_matches("r,");
                        if let Some((w, h)) = payload.split_once('x') {
                            if let (Ok(width), Ok(height)) = (w.parse::<u32>(), h.parse::<u32>()) {
                                if width > 0 && height > 0 && width <= 7680 && height <= 4320 {
                                    shared_state.resize_display(width, height);
                                }
                            }
                        }
                        return;
                    }
                    if text.starts_with("SET_NATIVE_CURSOR_RENDERING,") {
                        return;
                    }
                    if text.starts_with("_arg_fps,") {
                        let payload = text.trim_start_matches("_arg_fps,");
                        if let Ok(fps) = payload.parse::<u32>() {
                            runtime_settings.set_target_fps(fps);
                        }
                        return;
                    }
                    if text.starts_with("_f,") {
                        let payload = text.trim_start_matches("_f,");
                        if let Ok(fps) = payload.parse::<u32>() {
                            shared_state.update_client_fps(fps);
                        }
                        return;
                    }
                    if text.starts_with("_l,") {
                        let payload = text.trim_start_matches("_l,");
                        if let Ok(latency) = payload.parse::<u64>() {
                            shared_state.update_client_latency(latency);
                        }
                        return;
                    }
                    if text.starts_with("_stats_video,") {
                        let payload = text.trim_start_matches("_stats_video,");
                        shared_state.update_webrtc_stats("video", payload);
                        return;
                    }
                    if text.starts_with("_stats_audio,") {
                        let payload = text.trim_start_matches("_stats_audio,");
                        shared_state.update_webrtc_stats("audio", payload);
                        return;
                    }
                    match Self::parse_input_text(text) {
                        Ok(event) => {
                            if let Err(e) = input_tx.send(event) {
                                warn!("Failed to send input event: {}", e);
                            }
                        }
                        Err(e) => {
                            debug!("Failed to parse input from {}: {}", label, e);
                        }
                    }
                } else {
                    debug!("Ignoring non-text message on input channel {}", label);
                }
            })
        }));

        // Handle channel open
        self.channel.on_open(Box::new(move || {
            info!("Input DataChannel opened");
            Box::pin(async {})
        }));

        // Handle channel close
        self.channel.on_close(Box::new(move || {
            info!("Input DataChannel closed");
            Box::pin(async {})
        }));

        // Handle errors
        self.channel.on_error(Box::new(move |err| {
            error!("DataChannel error: {}", err);
            Box::pin(async {})
        }));
    }

    /// Parse an input message from DataChannel data
    ///
    /// Supports the same protocol as WebSocket:
    /// - Mouse move: `m,x,y` or `m,x,y,buttons`
    /// - Mouse button: `b,button,pressed` (pressed: 0 or 1)
    /// - Mouse wheel: `w,dx,dy`
    /// - Keyboard: `k,keysym,pressed`
    /// - Text input: `t,<utf8_text>`
    fn parse_input_message(data: &[u8]) -> Result<InputEventData, WebRTCError> {
        let text = std::str::from_utf8(data)
            .map_err(|e| WebRTCError::DataChannelError(format!("Invalid UTF-8: {}", e)))?;

        Self::parse_input_text(text)
    }

    /// Parse input text message
    pub fn parse_input_text(text: &str) -> Result<InputEventData, WebRTCError> {
        let parts: Vec<&str> = text.split(',').collect();

        if parts.is_empty() {
            return Err(WebRTCError::DataChannelError("Empty input message".to_string()));
        }

        let mut event = InputEventData::default();

        match parts[0] {
            // Mouse move: m,x,y or m,x,y,buttons
            "m" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid mouse move format".to_string()));
                }

                event.event_type = InputEvent::MouseMove;
                event.mouse_x = parts[1].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse X".to_string()))?;
                event.mouse_y = parts[2].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse Y".to_string()))?;

                // Optional button mask
                if parts.len() > 3 {
                    event.button_mask = parts[3].parse().unwrap_or(0);
                }
            }

            // Mouse button: b,button,pressed
            "b" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid mouse button format".to_string()));
                }

                event.event_type = InputEvent::MouseButton;
                event.mouse_button = parts[1].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid button number".to_string()))?;
                event.button_pressed = parts[2] == "1";
            }

            // Mouse wheel: w,dx,dy
            "w" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid mouse wheel format".to_string()));
                }

                event.event_type = InputEvent::MouseWheel;
                event.wheel_delta_x = parts[1].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid wheel delta X".to_string()))?;
                event.wheel_delta_y = parts[2].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid wheel delta Y".to_string()))?;
            }

            // Keyboard: k,keysym,pressed
            "k" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid keyboard format".to_string()));
                }

                event.event_type = InputEvent::Keyboard;

                // Parse keysym (can be hex with 0x prefix or decimal)
                let keysym_str = parts[1];
                event.keysym = if keysym_str.starts_with("0x") || keysym_str.starts_with("0X") {
                    u32::from_str_radix(&keysym_str[2..], 16)
                        .map_err(|_| WebRTCError::DataChannelError("Invalid hex keysym".to_string()))?
                } else {
                    keysym_str.parse()
                        .map_err(|_| WebRTCError::DataChannelError("Invalid keysym".to_string()))?
                };

                event.key_pressed = parts[2] == "1";
            }

            // Text input: t,<text>
            "t" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError("Invalid text input format".to_string()));
                }

                event.event_type = InputEvent::TextInput;
                // Rejoin text parts in case it contains commas
                event.text = parts[1..].join(",");
            }

            // Clipboard: c,<base64_text>
            "c" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError("Invalid clipboard format".to_string()));
                }

                event.event_type = InputEvent::Clipboard;
                event.text = parts[1..].join(",");
            }

            // Ping: p,timestamp
            "p" => {
                event.event_type = InputEvent::Ping;
                if parts.len() > 1 {
                    event.timestamp = parts[1].parse().unwrap_or(0);
                }
            }

            _ => {
                return Err(WebRTCError::DataChannelError(format!("Unknown input type: {}", parts[0])));
            }
        }

        Ok(event)
    }

    /// Send a message through the DataChannel
    pub async fn send(&self, data: &[u8]) -> Result<(), WebRTCError> {
        self.channel.send(&bytes::Bytes::copy_from_slice(data)).await
            .map_err(|e| WebRTCError::DataChannelError(format!("Send failed: {}", e)))?;
        Ok(())
    }

    /// Send a text message through the DataChannel
    pub async fn send_text(&self, text: &str) -> Result<(), WebRTCError> {
        self.send(text.as_bytes()).await
    }

    /// Check if the channel is open
    pub fn is_open(&self) -> bool {
        use webrtc::data_channel::data_channel_state::RTCDataChannelState;
        self.channel.ready_state() == RTCDataChannelState::Open
    }

    /// Get the channel label
    pub fn label(&self) -> String {
        self.channel.label().to_string()
    }

    /// Close the channel
    pub async fn close(&self) -> Result<(), WebRTCError> {
        self.channel.close().await
            .map_err(|e| WebRTCError::DataChannelError(format!("Close failed: {}", e)))?;
        Ok(())
    }
}

/// Auxiliary DataChannel handler (file uploads)
pub struct AuxDataChannel {
    channel: Arc<RTCDataChannel>,
    upload_handler: Arc<Mutex<FileUploadHandler>>,
}

impl AuxDataChannel {
    pub fn new(channel: Arc<RTCDataChannel>, upload_handler: Arc<Mutex<FileUploadHandler>>) -> Self {
        Self { channel, upload_handler }
    }

    pub async fn setup_handlers(&self) {
        let upload_handler = self.upload_handler.clone();
        let channel_label = self.channel.label().to_string();

        self.channel.on_message(Box::new(move |msg: DataChannelMessage| {
            let upload_handler = upload_handler.clone();
            let label = channel_label.clone();

            Box::pin(async move {
                if msg.is_string {
                    debug!("Ignoring text message on auxiliary channel {}", label);
                } else {
                    upload_handler.lock().unwrap_or_else(|e| e.into_inner()).handle_binary(&msg.data);
                }
            })
        }));

        self.channel.on_open(Box::new(move || {
            info!("Auxiliary DataChannel opened");
            Box::pin(async {})
        }));

        let upload_handler = self.upload_handler.clone();
        self.channel.on_close(Box::new(move || {
            info!("Auxiliary DataChannel closed");
            upload_handler.lock().unwrap_or_else(|e| e.into_inner()).abort_active();
            Box::pin(async {})
        }));

        self.channel.on_error(Box::new(move |err| {
            error!("Auxiliary DataChannel error: {}", err);
            Box::pin(async {})
        }));
    }
}

/// Format an outgoing message for the DataChannel
///
/// Supports:
/// - Cursor update: `cursor,<json>`
/// - Clipboard: `clipboard,<base64>`
/// - Stats: `stats,<json>`
/// - Pong: `pong,<timestamp>`
pub fn format_output_message(msg_type: &str, data: &str) -> String {
    format!("{},{}", msg_type, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mouse_move() {
        let event = InputDataChannel::parse_input_text("m,100,200").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseMove);
        assert_eq!(event.mouse_x, 100);
        assert_eq!(event.mouse_y, 200);
    }

    #[test]
    fn test_parse_mouse_move_with_buttons() {
        let event = InputDataChannel::parse_input_text("m,100,200,3").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseMove);
        assert_eq!(event.button_mask, 3);
    }

    #[test]
    fn test_parse_mouse_button() {
        let event = InputDataChannel::parse_input_text("b,1,1").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseButton);
        assert_eq!(event.mouse_button, 1);
        assert!(event.button_pressed);
    }

    #[test]
    fn test_parse_keyboard() {
        let event = InputDataChannel::parse_input_text("k,0xff08,1").unwrap();
        assert_eq!(event.event_type, InputEvent::Keyboard);
        assert_eq!(event.keysym, 0xff08);  // BackSpace
        assert!(event.key_pressed);
    }

    #[test]
    fn test_parse_keyboard_decimal() {
        let event = InputDataChannel::parse_input_text("k,65,1").unwrap();
        assert_eq!(event.event_type, InputEvent::Keyboard);
        assert_eq!(event.keysym, 65);  // 'A'
        assert!(event.key_pressed);
    }

    #[test]
    fn test_parse_wheel() {
        let event = InputDataChannel::parse_input_text("w,0,-120").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseWheel);
        assert_eq!(event.wheel_delta_x, 0);
        assert_eq!(event.wheel_delta_y, -120);
    }

    #[test]
    fn test_parse_text_with_comma() {
        let event = InputDataChannel::parse_input_text("t,hello,world").unwrap();
        assert_eq!(event.event_type, InputEvent::TextInput);
        assert_eq!(event.text, "hello,world");
    }
}
