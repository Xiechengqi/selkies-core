//! WebRTC Signaling Server (str0m)
//!
//! Handles WebRTC signaling over WebSocket connections.
//! With str0m, the flow is:
//! 1. Browser sends SDP offer via WebSocket
//! 2. Server creates str0m Rtc, accepts offer, returns answer with TCP candidate
//! 3. Browser connects via ICE-TCP to the same port
//! 4. TCP protocol splitter routes the connection to the matching session

#![allow(dead_code)]

use crate::webrtc::{SignalingMessage, SessionManager};
use crate::webrtc::signaling::SignalingParser;
use crate::web::SharedState;
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use log::{info, warn, debug, error};
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Signaling server configuration
#[derive(Debug, Clone)]
pub struct SignalingConfig {
    /// WebSocket endpoint path
    pub path: String,
    /// Ping interval in seconds
    pub ping_interval_secs: u64,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
}

impl Default for SignalingConfig {
    fn default() -> Self {
        Self {
            path: "/webrtc".to_string(),
            ping_interval_secs: 30,
            timeout_secs: 60,
        }
    }
}

/// Handle a WebRTC signaling WebSocket connection
pub async fn handle_signaling_connection(
    socket: WebSocket,
    state: Arc<SharedState>,
    session_manager: Arc<SessionManager>,
) {
    info!("New signaling WebSocket connection established");
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Create a channel for sending messages
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Spawn task to forward messages to WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Session ID for this connection
    let mut session_id: Option<String> = None;
    let mut wire_format = WireFormat::Selkies;

    // Process incoming messages
    while let Some(result) = ws_receiver.next().await {
        match result {
            Ok(Message::Text(text)) => {
                let text_str: &str = text.as_ref();

                if let Some(reply) = handle_gstreamer_control_message(text_str, &mut wire_format) {
                    let _ = tx.send(reply);
                    continue;
                }

                if let Some(msg) = parse_gstreamer_json_message(text_str) {
                    wire_format = WireFormat::GStreamer;
                    if let Some(response) = handle_signaling_message(
                        msg,
                        &mut session_id,
                        &state,
                        &session_manager,
                        &tx,
                        wire_format,
                    ).await {
                        let _ = tx.send(response);
                    }
                    continue;
                }

                match SignalingParser::parse(text_str) {
                    Ok(msg) => {
                        if let Some(response) = handle_signaling_message(
                            msg,
                            &mut session_id,
                            &state,
                            &session_manager,
                            &tx,
                            wire_format,
                        ).await {
                            let _ = tx.send(response);
                        }
                    }
                    Err(e) => {
                        warn!("Invalid signaling message: {}", e);
                        let error = SignalingMessage::error(
                            "PARSE_ERROR",
                            &e.to_string(),
                            session_id.clone(),
                        );
                        if let Some(msg) = format_signaling_message(&error, wire_format) {
                            let _ = tx.send(msg);
                        }
                    }
                }
            }
            Ok(Message::Binary(_)) => {
                debug!("Received binary message on signaling channel");
            }
            Ok(Message::Ping(_data)) => {
                debug!("Received ping on signaling channel");
            }
            Ok(Message::Close(_)) => {
                info!("Signaling connection closed");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    // Clean up send task
    send_task.abort();

    // Clean up pending session if the WebSocket closes before ICE-TCP connects
    if let Some(ref sid) = session_id {
        session_manager.remove_pending_session(sid).await;
    }

    info!("Signaling connection handler finished (session: {:?})", session_id);
}

/// Handle a single signaling message.
///
/// With str0m, the signaling flow is simpler:
/// - Offer → create_session_with_offer() → returns answer with TCP candidate
/// - No ICE trickle needed (server injects a single TCP passive candidate)
/// - No ICE candidate forwarding from server to browser
/// - Browser ICE candidates are ignored (ICE-lite server doesn't need them)
async fn handle_signaling_message(
    message: SignalingMessage,
    session_id: &mut Option<String>,
    state: &Arc<SharedState>,
    session_manager: &Arc<SessionManager>,
    tx: &mpsc::UnboundedSender<String>,
    wire_format: WireFormat,
) -> Option<String> {
    match message {
        SignalingMessage::Offer { sdp, session_id: provided_session_id } => {
            // Create session and accept offer in one step
            match session_manager.create_session_with_offer(&sdp).await {
                Ok((sid, answer_sdp)) => {
                    *session_id = Some(sid.clone());
                    info!("Session {} created with SDP answer", sid);

                    // Send ready notification (Selkies format)
                    if wire_format == WireFormat::Selkies {
                        let ready = SignalingMessage::ready(
                            sid.clone(),
                            session_manager.config().video_codec.as_str(),
                            "input",
                        );
                        if let Some(payload) = format_signaling_message(&ready, wire_format) {
                            let _ = tx.send(payload);
                        }
                    }

                    // Send ICE gathering complete (no trickle needed with ICE-lite TCP)
                    if wire_format == WireFormat::Selkies {
                        let complete = SignalingMessage::IceComplete {
                            session_id: sid.clone(),
                        };
                        if let Some(payload) = format_signaling_message(&complete, wire_format) {
                            let _ = tx.send(payload);
                        }
                    }

                    let answer = SignalingMessage::answer(answer_sdp, sid);
                    format_signaling_message(&answer, wire_format)
                }
                Err(e) => {
                    error!("Failed to create session: {}", e);
                    let error = SignalingMessage::error(
                        "SESSION_ERROR",
                        &e.to_string(),
                        provided_session_id,
                    );
                    format_signaling_message(&error, wire_format)
                }
            }
        }

        SignalingMessage::Answer { sdp: _, session_id: _msg_session_id } => {
            // str0m ICE-lite server doesn't process answers from the browser
            // (we already generated the answer). Log and ignore.
            debug!("Ignoring SDP answer from browser (ICE-lite server)");
            None
        }

        SignalingMessage::IceCandidate { candidate, sdp_mid: _, sdp_mline_index: _, session_id: _ } => {
            // With ICE-lite, we don't need remote candidates from the browser.
            // The browser will connect to our TCP passive candidate directly.
            // Record for metrics but don't process.
            let (transport, candidate_type) = parse_ice_candidate(&candidate);
            state.record_ice_candidate(transport.as_deref(), candidate_type.as_deref());
            debug!("Received browser ICE candidate (ignored in ICE-lite mode): {}", &candidate[..candidate.len().min(80)]);
            None
        }

        SignalingMessage::KeyframeRequest { session_id: msg_session_id } => {
            let target = session_id.as_deref().unwrap_or(&msg_session_id);
            debug!("Keyframe requested for session {}", target);
            state.runtime_settings.request_keyframe();
            None
        }

        SignalingMessage::BitrateRequest { session_id: msg_session_id, bitrate_kbps } => {
            let target = session_id.as_deref().unwrap_or(&msg_session_id);
            debug!("Bitrate change requested for session {}: {} kbps", target, bitrate_kbps);
            state.runtime_settings.set_video_bitrate_kbps(bitrate_kbps);
            None
        }

        SignalingMessage::Ping { timestamp } => {
            let pong = SignalingMessage::Pong { timestamp };
            format_signaling_message(&pong, wire_format)
        }

        SignalingMessage::Close { session_id: msg_session_id, reason } => {
            let target = session_id.clone().unwrap_or(msg_session_id);
            info!("Session close requested: {} (reason: {:?})", target, reason);
            session_manager.remove_pending_session(&target).await;
            *session_id = None;
            None
        }

        _ => {
            debug!("Unhandled signaling message type");
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireFormat {
    Selkies,
    GStreamer,
}

fn handle_gstreamer_control_message(text: &str, wire_format: &mut WireFormat) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.starts_with("HELLO") {
        *wire_format = WireFormat::GStreamer;
        return Some("HELLO".to_string());
    }
    if trimmed.starts_with("SESSION") {
        *wire_format = WireFormat::GStreamer;
        return Some("SESSION_OK".to_string());
    }
    None
}

fn parse_gstreamer_json_message(text: &str) -> Option<SignalingMessage> {
    let value: Value = serde_json::from_str(text).ok()?;
    if let Some(sdp) = value.get("sdp") {
        let sdp_obj = sdp.as_object()?;
        let sdp_type = sdp_obj.get("type")?.as_str()?;
        let sdp_text = sdp_obj.get("sdp")?.as_str()?;
        if sdp_type == "offer" {
            return Some(SignalingMessage::Offer {
                sdp: sdp_text.to_string(),
                session_id: None,
            });
        }
        if sdp_type == "answer" {
            return Some(SignalingMessage::Answer {
                sdp: sdp_text.to_string(),
                session_id: String::default(),
            });
        }
        return None;
    }
    if let Some(ice) = value.get("ice") {
        let ice_obj = ice.as_object()?;
        let candidate = ice_obj.get("candidate")?.as_str()?.to_string();
        let sdp_mid = ice_obj.get("sdpMid").and_then(|v| v.as_str()).map(|s| s.to_string());
        let sdp_mline_index = ice_obj.get("sdpMLineIndex").and_then(|v| v.as_u64()).map(|v| v as u16);
        return Some(SignalingMessage::IceCandidate {
            candidate,
            sdp_mid,
            sdp_mline_index,
            session_id: String::new(),
        });
    }
    None
}

fn format_signaling_message(message: &SignalingMessage, wire_format: WireFormat) -> Option<String> {
    match wire_format {
        WireFormat::Selkies => message.to_json().ok(),
        WireFormat::GStreamer => match message {
            SignalingMessage::Offer { sdp, .. } => {
                Some(json!({ "sdp": { "type": "offer", "sdp": sdp } }).to_string())
            }
            SignalingMessage::Answer { sdp, .. } => {
                Some(json!({ "sdp": { "type": "answer", "sdp": sdp } }).to_string())
            }
            SignalingMessage::IceCandidate { candidate, sdp_mid, sdp_mline_index, .. } => {
                let payload = json!({
                    "ice": {
                        "candidate": candidate,
                        "sdpMid": sdp_mid,
                        "sdpMLineIndex": sdp_mline_index
                    }
                });
                Some(payload.to_string())
            }
            SignalingMessage::IceComplete { .. } => None,
            SignalingMessage::Error { code, message, .. } => {
                Some(format!("ERROR {}: {}", code, message))
            }
            SignalingMessage::Pong { timestamp } => {
                Some(json!({ "pong": timestamp }).to_string())
            }
            _ => None,
        },
    }
}

fn parse_ice_candidate(candidate: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = candidate.split_whitespace().collect();
    if parts.len() < 8 {
        return (None, None);
    }

    let transport = parts.get(2).map(|v| v.to_ascii_lowercase());
    let mut candidate_type = None;
    if let Some(idx) = parts.iter().position(|p| *p == "typ") {
        if let Some(typ) = parts.get(idx + 1) {
            candidate_type = Some(typ.to_ascii_lowercase());
        }
    }

    (transport, candidate_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signaling_config_default() {
        let config = SignalingConfig::default();
        assert_eq!(config.path, "/webrtc");
        assert_eq!(config.ping_interval_secs, 30);
    }
}
