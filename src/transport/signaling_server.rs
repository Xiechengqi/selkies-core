//! WebRTC Signaling Server
//!
//! Handles WebRTC signaling over WebSocket connections.
//! Provides SDP offer/answer exchange and ICE candidate transmission.

#![allow(dead_code)]

use crate::webrtc::{
    SignalingMessage, SessionManager,
};
use crate::webrtc::signaling::SignalingParser;
use crate::webrtc::peer_connection::PeerConnectionManager;
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
                    // Send the control reply first (HELLO or SESSION_OK)
                    let _ = tx.send(reply.clone());
                    if reply == "SESSION_OK" {
                        if let Some(payload) = handle_gstreamer_session_request(
                            &mut session_id,
                            &state,
                            &session_manager,
                            &tx,
                        )
                        .await
                        {
                            let _ = tx.send(payload);
                        }
                    }
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
                    )
                    .await
                    {
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
                        )
                        .await
                        {
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
                // Binary messages not used for signaling
                debug!("Received binary message on signaling channel");
            }
            Ok(Message::Ping(_data)) => {
                // Respond to ping
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

    // Clean up session if created
    if let Some(id) = session_id {
        session_manager.remove_session(&id).await;
        info!("Cleaned up WebRTC session: {}", id);
    }

    // Clean up send task
    send_task.abort();
}

/// Handle a single signaling message
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
            // Create a new session if none exists
            let session = if let Some(ref id) = *session_id {
                session_manager.get_session(id).await
            } else {
                match session_manager.create_session().await {
                    Ok(s) => {
                        *session_id = Some(s.id.clone());
                        Some(s)
                    }
                    Err(e) => {
                        error!("Failed to create session: {}", e);
                        let error = SignalingMessage::error(
                            "SESSION_ERROR",
                            &e.to_string(),
                            provided_session_id,
                        );
                        return error.to_json().ok();
                    }
                }
            };

            let session = match session {
                Some(s) => s,
                None => {
                    let error = SignalingMessage::error(
                        "SESSION_NOT_FOUND",
                        "Session not found",
                        session_id.clone(),
                    );
                    return error.to_json().ok();
                }
            };

            // Handle the offer
            match session_manager.handle_offer(&session.id, &sdp).await {
                Ok(answer_sdp) => {
                    let answer = SignalingMessage::answer(answer_sdp, session.id.clone());

                    if session_manager.config().ice_trickle {
                        // Set up ICE candidate forwarding
                        let tx_clone = tx.clone();
                        let session_id_clone = session.id.clone();
                        let state_clone = state.clone();
                        let format_clone = wire_format;

                        crate::webrtc::peer_connection::PeerConnectionManager::setup_ice_callback(
                            &session.peer_connection,
                            move |candidate_opt| {
                                if let Some(candidate) = candidate_opt {
                                    let (transport, candidate_type) = parse_ice_candidate(&candidate);
                                    state_clone.record_ice_candidate(transport.as_deref(), candidate_type.as_deref());
                                    let msg = SignalingMessage::ice_candidate(
                                        candidate,
                                        Some("0".to_string()),
                                        Some(0),
                                        session_id_clone.clone(),
                                    );
                                    if let Some(payload) = format_signaling_message(&msg, format_clone) {
                                        let _ = tx_clone.send(payload);
                                    }
                                } else {
                                    // ICE gathering complete
                                    let msg = SignalingMessage::IceComplete {
                                        session_id: session_id_clone.clone(),
                                    };
                                    if let Some(payload) = format_signaling_message(&msg, format_clone) {
                                        let _ = tx_clone.send(payload);
                                    }
                                }
                            },
                        ).await;
                    }

                    // Send ready notification
                    if wire_format == WireFormat::Selkies {
                        let ready = SignalingMessage::ready(
                            session.id.clone(),
                            session_manager.config().video_codec.as_str(),
                            "input",
                        );
                        if let Some(payload) = format_signaling_message(&ready, wire_format) {
                            let _ = tx.send(payload);
                        }
                    }

                    format_signaling_message(&answer, wire_format)
                }
                Err(e) => {
                    error!("Failed to handle offer: {}", e);
                    let error = SignalingMessage::error(
                        "OFFER_ERROR",
                        &e.to_string(),
                        Some(session.id.clone()),
                    );
                    format_signaling_message(&error, wire_format)
                }
            }
        }

        SignalingMessage::Answer { sdp, session_id: msg_session_id } => {
            let target_session_id = session_id.clone().unwrap_or(msg_session_id);
            let session = match session_manager.get_session(&target_session_id).await {
                Some(s) => s,
                None => {
                    let error = SignalingMessage::error(
                        "SESSION_NOT_FOUND",
                        "Session not found",
                        Some(target_session_id),
                    );
                    return format_signaling_message(&error, wire_format);
                }
            };

            if let Err(e) = PeerConnectionManager::handle_answer(&session.peer_connection, &sdp).await {
                let error = SignalingMessage::error("ANSWER_ERROR", &e.to_string(), Some(session.id.clone()));
                return format_signaling_message(&error, wire_format);
            }

            session.touch().await;
            None
        }

        SignalingMessage::IceCandidate { candidate, sdp_mid, sdp_mline_index, session_id: msg_session_id } => {
            let target_session_id = session_id.clone().unwrap_or(msg_session_id);

            if let Err(e) = session_manager.add_ice_candidate(
                &target_session_id,
                &candidate,
                sdp_mid.as_deref(),
                sdp_mline_index,
            ).await {
                warn!("Failed to add ICE candidate: {}", e);
            }
            None
        }

        SignalingMessage::KeyframeRequest { session_id: msg_session_id } => {
            let target_session_id = session_id.clone().unwrap_or(msg_session_id);
            debug!("Keyframe requested for session {}", target_session_id);
            state.runtime_settings.request_keyframe();
            None
        }

        SignalingMessage::BitrateRequest { session_id: msg_session_id, bitrate_kbps } => {
            let target_session_id = session_id.clone().unwrap_or(msg_session_id);
            debug!("Bitrate change requested for session {}: {} kbps", target_session_id, bitrate_kbps);
            state.runtime_settings.set_video_bitrate_kbps(bitrate_kbps);
            None
        }

        SignalingMessage::Ping { timestamp } => {
            let pong = SignalingMessage::Pong { timestamp };
            format_signaling_message(&pong, wire_format)
        }

        SignalingMessage::Close { session_id: msg_session_id, reason } => {
            let target_session_id = session_id.clone().unwrap_or(msg_session_id);
            info!("Session close requested: {} (reason: {:?})", target_session_id, reason);
            session_manager.remove_session(&target_session_id).await;
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

async fn handle_gstreamer_session_request(
    session_id: &mut Option<String>,
    state: &Arc<SharedState>,
    session_manager: &Arc<SessionManager>,
    tx: &mpsc::UnboundedSender<String>,
) -> Option<String> {
    if session_id.is_none() {
        match session_manager.create_session().await {
            Ok(session) => {
                *session_id = Some(session.id.clone());

                // Set up ICE forwarding (trickle)
                let tx_clone = tx.clone();
                let session_id_clone = session.id.clone();
                let state_clone = state.clone();
                if session_manager.config().ice_trickle {
                    PeerConnectionManager::setup_ice_callback(
                        &session.peer_connection,
                        move |candidate_opt| {
                            if let Some(candidate) = candidate_opt {
                                let (transport, candidate_type) = parse_ice_candidate(&candidate);
                                state_clone.record_ice_candidate(transport.as_deref(), candidate_type.as_deref());
                                let msg = SignalingMessage::ice_candidate(
                                    candidate,
                                    Some("0".to_string()),
                                    Some(0),
                                    session_id_clone.clone(),
                                );
                                if let Some(payload) = format_signaling_message(&msg, WireFormat::GStreamer) {
                                    let _ = tx_clone.send(payload);
                                }
                            } else {
                                let msg = SignalingMessage::IceComplete {
                                    session_id: session_id_clone.clone(),
                                };
                                if let Some(payload) = format_signaling_message(&msg, WireFormat::GStreamer) {
                                    let _ = tx_clone.send(payload);
                                }
                            }
                        },
                    )
                    .await;
                }

                // Create and send offer
                if let Ok(offer_sdp) = PeerConnectionManager::create_offer(&session.peer_connection).await {
                    let offer = SignalingMessage::Offer {
                        sdp: offer_sdp,
                        session_id: Some(session.id.clone()),
                    };
                    return format_signaling_message(&offer, WireFormat::GStreamer);
                }
            }
            Err(e) => {
                let error = SignalingMessage::error("SESSION_ERROR", &e.to_string(), None);
                return format_signaling_message(&error, WireFormat::GStreamer);
            }
        }
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

    // candidate:<foundation> <component> <transport> <priority> <ip> <port> typ <type> ...
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
