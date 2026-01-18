//! WebSocket server implementation
//!
//! Manages WebSocket connections and message routing.

use crate::encode::Stripe;
use crate::web::shared::SharedState;
use base64::Engine;
use futures::{SinkExt, StreamExt};
use log::{debug, error, info};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::time::{self, Duration};
use tokio_tungstenite::tungstenite::protocol::Message;
use std::time::{SystemTime, UNIX_EPOCH};

/// WebSocket server
pub struct WebSocketServer {
    /// Bind address
    addr: String,
    /// Port
    port: u16,
    /// Shared state
    state: Arc<SharedState>,
}

impl WebSocketServer {
    /// Create a new WebSocket server
    pub fn new(addr: String, port: u16, state: Arc<SharedState>) -> Self {
        Self {
            addr,
            port,
            state,
        }
    }

    /// Run the WebSocket server
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(format!("{}:{}", self.addr, self.port)).await?;
        info!("WebSocket server listening on ws://{}:{}", self.addr, self.port);

        loop {
            let (stream, addr) = listener.accept().await?;
            info!("New connection from {}", addr);

            let state = self.state.clone();
            let frame_receiver = state.frame_sender.subscribe();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, addr, state, frame_receiver).await {
                    error!("Connection error: {}", e);
                }
            });
        }
    }
}

/// Handle a WebSocket connection
async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    state: Arc<SharedState>,
    mut frame_receiver: broadcast::Receiver<Vec<Stripe>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let ws_stream = tokio_tungstenite::accept_async(stream).await?;

    info!("WebSocket handshake completed for {}", addr);

    let (write, mut read) = ws_stream.split();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Message>();
    let outbound_tx_frames = outbound_tx.clone();
    let outbound_tx_text = outbound_tx.clone();
    let outbound_tx_audio = outbound_tx.clone();

    let writer_handle = tokio::spawn(async move {
        let mut write = write;
        while let Some(msg) = outbound_rx.recv().await {
            if write.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Add client to state
    let client_id = state.add_client(addr);

    // Send handshake messages
    let (display_width, display_height) = state.display_size();
    let _ = outbound_tx.send(Message::Text("MODE websockets".into()));
    let resolution = format!("system,{}", system_json("resolution", &format!("{}x{}", display_width, display_height)));
    let framerate = format!("system,{}", system_json("framerate", &state.config.encoding.target_fps.to_string()));
    let encoder = format!("system,{}", system_json("encoder", "jpeg"));
    let ui_config = format!("system,{}", system_json_raw("ui_config", &state.ui_config_json()));
    let _ = outbound_tx.send(Message::Text(resolution.into()));
    let _ = outbound_tx.send(Message::Text(framerate.into()));
    let _ = outbound_tx.send(Message::Text(encoder.into()));
    let _ = outbound_tx.send(Message::Text(ui_config.into()));
    let _ = outbound_tx.send(Message::Text(server_settings_json(&state).into()));
    let _ = outbound_tx.send(Message::Text("VIDEO_STARTED".into()));
    if let Some(cursor_message) = state.last_cursor_message() {
        let _ = outbound_tx.send(Message::Text(cursor_message.into()));
    }

    // Forward text broadcasts (clipboard, stats, system)
    let mut text_receiver = state.text_sender.subscribe();
    tokio::spawn(async move {
        while let Ok(message) = text_receiver.recv().await {
            if outbound_tx_text.send(Message::Text(message.into())).is_err() {
                break;
            }
        }
    });

    // Forward audio packets
    let mut audio_receiver = state.audio_sender.subscribe();
    tokio::spawn(async move {
        while let Ok(packet) = audio_receiver.recv().await {
            let mut payload = Vec::with_capacity(2 + packet.data.len());
            payload.push(0x01);
            payload.push(0x00);
            payload.extend_from_slice(&packet.data);
            if outbound_tx_audio.send(Message::Binary(payload)).is_err() {
                break;
            }
        }
    });

    // Forward video frames to client
    let state_for_frames = state.clone();
    tokio::spawn(async move {
        let mut frame_count = 0u64;
        let mut first_frame_skipped = false;

        while let Ok(stripes) = frame_receiver.recv().await {
            // Skip the first frame and request full refresh
            if !first_frame_skipped {
                first_frame_skipped = true;
                log::info!("Skipping first frame, requesting full refresh");
                state_for_frames.request_full_refresh();
                continue;
            }

            if frame_count == 0 {
                log::info!("Sending first frame with {} stripes to client", stripes.len());
            }
            frame_count += 1;
            for stripe in stripes {
                let mut payload = Vec::with_capacity(6 + stripe.data.len());
                payload.push(0x03);
                payload.push(0x00);
                payload.extend_from_slice(&stripe.frame_id.to_be_bytes());
                let y = stripe.y.min(u16::MAX as u32) as u16;
                payload.extend_from_slice(&y.to_be_bytes());
                payload.extend_from_slice(&stripe.data);
                if outbound_tx_frames.send(Message::Binary(payload)).is_err() {
                    break;
                }
            }
        }
    });

    // Send periodic ping messages
    let outbound_tx_ping = outbound_tx.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let ping = format!("ping,{:.3}", timestamp);
            if outbound_tx_ping.send(Message::Text(ping.into())).is_err() {
                break;
            }
        }
    });

    // Handle incoming messages
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = handle_message(&text, &state, &client_id, &outbound_tx).await {
                    error!("Message handling error: {}", e);
                }
            }
            Ok(Message::Binary(data)) => {
                debug!("Received binary message: {} bytes", data.len());
            }
            Ok(Message::Ping(ping)) => {
                let _ = outbound_tx.send(Message::Pong(ping));
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Frame(_)) => {}
            Ok(Message::Close(_)) => break,
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
        }
    }

    // Remove client from state
    state.remove_client(&client_id);
    info!("Client {} disconnected", addr);

    drop(outbound_tx);
    let _ = writer_handle.await;
    Ok(())
}

fn system_json(action: &str, data: &str) -> String {
    format!(r#"{{"action":"{}","data":"{}"}}"#, action, data)
}

fn system_json_raw(action: &str, raw_json: &str) -> String {
    format!(r#"{{"action":"{}","data":{}}}"#, action, raw_json)
}

fn server_settings_json(state: &SharedState) -> String {
    let ui = &state.ui_config;
    let mut settings = serde_json::Map::new();
    settings.insert(
        "encoder".to_string(),
        json!({
            "allowed": ["jpeg"],
            "value": "jpeg"
        }),
    );
    settings.insert(
        "framerate".to_string(),
        json!({
            "min": ui.video.framerate.min,
            "max": ui.video.framerate.max,
            "default": ui.video.framerate.value
        }),
    );
    settings.insert(
        "jpeg_quality".to_string(),
        json!({
            "min": ui.video.jpeg_quality.min,
            "max": ui.video.jpeg_quality.max,
            "default": ui.video.jpeg_quality.value
        }),
    );
    settings.insert(
        "paint_over_jpeg_quality".to_string(),
        json!({
            "min": 1,
            "max": 100,
            "default": 90
        }),
    );
    settings.insert(
        "audio_bitrate".to_string(),
        json!({
            "min": ui.audio.bitrate.min,
            "max": ui.audio.bitrate.max,
            "default": ui.audio.bitrate.value
        }),
    );
    settings.insert(
        "is_manual_resolution_mode".to_string(),
        json!({
            "value": ui.screen.manual_resolution.value,
            "locked": ui.screen.manual_resolution.locked
        }),
    );
    if ui.screen.manual_resolution.value {
        settings.insert(
            "manual_width".to_string(),
            json!({
                "min": ui.screen.width.value,
                "max": ui.screen.width.value,
                "default": ui.screen.width.value
            }),
        );
        settings.insert(
            "manual_height".to_string(),
            json!({
                "min": ui.screen.height.value,
                "max": ui.screen.height.value,
                "default": ui.screen.height.value
            }),
        );
    }

    json!({
        "type": "server_settings",
        "settings": settings
    })
    .to_string()
}

/// Handle a client message
async fn handle_message(
    text: &str,
    state: &SharedState,
    client_id: &str,
    outbound_tx: &mpsc::UnboundedSender<Message>,
) -> Result<(), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = text.split(',').collect();

    if parts.is_empty() {
        return Ok(());
    }

    match parts[0] {
        "m" | "m2" => {
            // Mouse move: m,{x},{y}[,{mask},{unused}]
            // Relative move: m2,{dx},{dy}[,{mask},{unused}]
            if parts.len() >= 3 {
                let x: i32 = parts[1].parse()?;
                let y: i32 = parts[2].parse()?;
                let is_relative = parts[0] == "m2";
                let (new_x, new_y) = {
                    let mut positions = state.client_mouse_positions.lock().unwrap();
                    let (prev_x, prev_y) = positions
                        .get(client_id)
                        .copied()
                        .unwrap_or((0, 0));
                    let (next_x, next_y) = if is_relative {
                        (prev_x.saturating_add(x), prev_y.saturating_add(y))
                    } else {
                        (x, y)
                    };
                    positions.insert(client_id.to_string(), (next_x, next_y));
                    (next_x, next_y)
                };
                state.inject_mouse_move(client_id, new_x, new_y);

                if parts.len() >= 4 {
                    if let Ok(mask_val) = parts[3].parse::<u32>() {
                        let mask = (mask_val & 0xFF) as u8;
                        let prev_mask = {
                            let mut masks = state.client_button_masks.lock().unwrap();
                            let prev = masks.get(client_id).copied().unwrap_or(0);
                            masks.insert(client_id.to_string(), mask);
                            prev
                        };
                        if prev_mask != mask {
                            let button_map: &[(u8, u8)] = &[
                                (0x01, 1),
                                (0x02, 2),
                                (0x04, 3),
                                (0x08, 4),
                                (0x10, 5),
                                (0x20, 6),
                                (0x40, 7),
                                (0x80, 8),
                            ];
                            for (bit, button) in button_map.iter().copied() {
                                let was_down = (prev_mask & bit) != 0;
                                let is_down = (mask & bit) != 0;
                                if was_down != is_down {
                                    state.inject_mouse_button(client_id, button, is_down);
                                }
                            }
                        }
                    }
                }
            }
        }
        "b" | "M" => {
            // Mouse button: b,{button},{pressed}
            if parts.len() >= 3 {
                let button: u8 = parts[1].parse()?;
                let pressed = parse_bool(parts[2])?;
                state.inject_mouse_button(client_id, button, pressed);
                let bit = match button {
                    1 => 0x01,
                    2 => 0x02,
                    3 => 0x04,
                    4 => 0x08,
                    5 => 0x10,
                    6 => 0x20,
                    7 => 0x40,
                    8 => 0x80,
                    _ => 0,
                };
                if bit != 0 {
                    let mut masks = state.client_button_masks.lock().unwrap();
                    let prev = masks.get(client_id).copied().unwrap_or(0);
                    let next = if pressed { prev | bit } else { prev & !bit };
                    masks.insert(client_id.to_string(), next);
                }
            }
        }
        "w" | "W" => {
            // Mouse wheel: w,{delta_x},{delta_y}
            if parts.len() >= 3 {
                let dx: i32 = parts[1].parse()?;
                let dy: i32 = parts[2].parse()?;
                let dx = dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                let dy = dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                state.inject_mouse_wheel(client_id, dx, dy);
            }
        }
        "k" => {
            // Keyboard: k,{keysym},{pressed}
            if parts.len() >= 3 {
                let keysym: u32 = parts[1].parse()?;
                let pressed = parse_bool(parts[2])?;
                state.inject_keyboard(client_id, keysym, pressed);
            }
        }
        "c" => {
            // Clipboard: c,{base64_text}
            if parts.len() >= 2 && state.config.input.enable_clipboard {
                let decoded = base64::engine::general_purpose::STANDARD.decode(parts[1])?;
                let normalized = base64::engine::general_purpose::STANDARD.encode(decoded);
                state.set_clipboard(normalized);
            }
        }
        "pong" => {
            if parts.len() >= 2 {
                let sent_ts: f64 = parts[1].parse()?;
                let now_ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(sent_ts);
                let latency_ms = ((now_ts - sent_ts) * 1000.0).max(0.0) as u64;
                state.update_latency(latency_ms);
                let payload = format!("system,{}", system_json("latency", &latency_ms.to_string()));
                let _ = outbound_tx.send(Message::Text(payload.into()));
            }
        }
        "_r" => {
            if parts.len() >= 2 {
                match parts[1] {
                    "requestFullRefresh" => state.request_full_refresh(),
                    "requestStats" => {
                        let stats = state.stats_json();
                        let _ = outbound_tx.send(Message::Text(format!("stats,{}", stats).into()));
                    }
                    "requestReload" => {
                        let _ = outbound_tx.send(Message::Text("system,reload".into()));
                    }
                    _ => {}
                }
            }
        }
        _ => {
            debug!("Unknown message type: {}", parts[0]);
        }
    }

    Ok(())
}

fn parse_bool(value: &str) -> Result<bool, Box<dyn std::error::Error>> {
    match value {
        "1" | "true" | "True" => Ok(true),
        "0" | "false" | "False" => Ok(false),
        _ => Err(format!("Invalid boolean value: {}", value).into()),
    }
}
