//! WebRTC Signaling Server
//!
//! Handles WebRTC signaling over WebSocket connections.
//! Provides SDP offer/answer exchange and ICE candidate transmission.

#![allow(dead_code)]

use crate::webrtc::{
    SignalingMessage, SessionManager,
};
use crate::webrtc::signaling::SignalingParser;
use crate::web::SharedState;
use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use log::{info, warn, debug, error};
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

    // Process incoming messages
    while let Some(result) = ws_receiver.next().await {
        match result {
            Ok(Message::Text(text)) => {
                let text_str: &str = text.as_ref();
                match SignalingParser::parse(text_str) {
                    Ok(msg) => {
                        if let Some(response) = handle_signaling_message(
                            msg,
                            &mut session_id,
                            &state,
                            &session_manager,
                            &tx,
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
                        if let Ok(json) = error.to_json() {
                            let _ = tx.send(json);
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
                                    if let Ok(json) = msg.to_json() {
                                        let _ = tx_clone.send(json);
                                    }
                                } else {
                                    // ICE gathering complete
                                    let msg = SignalingMessage::IceComplete {
                                        session_id: session_id_clone.clone(),
                                    };
                                    if let Ok(json) = msg.to_json() {
                                        let _ = tx_clone.send(json);
                                    }
                                }
                            },
                        ).await;
                    }

                    // Send ready notification
                    let ready = SignalingMessage::ready(
                        session.id.clone(),
                        session_manager.config().video_codec.as_str(),
                        "input",
                    );
                    if let Ok(json) = ready.to_json() {
                        let _ = tx.send(json);
                    }

                    answer.to_json().ok()
                }
                Err(e) => {
                    error!("Failed to handle offer: {}", e);
                    let error = SignalingMessage::error(
                        "OFFER_ERROR",
                        &e.to_string(),
                        Some(session.id.clone()),
                    );
                    error.to_json().ok()
                }
            }
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
            pong.to_json().ok()
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
