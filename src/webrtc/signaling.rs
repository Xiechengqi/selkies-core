//! WebRTC Signaling Protocol
//!
//! Handles SDP offer/answer exchange and ICE candidate transmission
//! over WebSocket for WebRTC connection establishment.

#![allow(dead_code)]

use super::WebRTCError;
use serde::{Deserialize, Serialize};

/// Signaling message types for WebRTC negotiation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SignalingMessage {
    /// SDP Offer from client
    Offer {
        sdp: String,
        #[serde(default)]
        session_id: Option<String>,
    },

    /// SDP Answer from server
    Answer {
        sdp: String,
        session_id: String,
    },

    /// ICE Candidate
    IceCandidate {
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: Option<String>,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: Option<u16>,
        session_id: String,
    },

    /// ICE gathering complete (empty candidate)
    IceComplete {
        session_id: String,
    },

    /// Session ready notification
    Ready {
        session_id: String,
        video_codec: String,
        #[serde(rename = "dataChannel")]
        data_channel: String,
    },

    /// Error message
    Error {
        code: String,
        message: String,
        #[serde(default)]
        session_id: Option<String>,
    },

    /// Ping/keepalive
    Ping {
        timestamp: u64,
    },

    /// Pong response
    Pong {
        timestamp: u64,
    },

    /// Request keyframe
    KeyframeRequest {
        session_id: String,
    },

    /// Bitrate change request
    BitrateRequest {
        session_id: String,
        bitrate_kbps: u32,
    },

    /// Session statistics
    Stats {
        session_id: String,
        #[serde(rename = "roundTripTime")]
        round_trip_time_ms: Option<f64>,
        #[serde(rename = "packetsLost")]
        packets_lost: Option<u64>,
        #[serde(rename = "jitter")]
        jitter_ms: Option<f64>,
    },

    /// Close session
    Close {
        session_id: String,
        reason: Option<String>,
    },
}

impl SignalingMessage {
    /// Parse a signaling message from JSON
    pub fn from_json(json: &str) -> Result<Self, WebRTCError> {
        serde_json::from_str(json)
            .map_err(|e| WebRTCError::SdpError(format!("Invalid signaling message: {}", e)))
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, WebRTCError> {
        serde_json::to_string(self)
            .map_err(|e| WebRTCError::SdpError(format!("Failed to serialize message: {}", e)))
    }

    /// Create an error response
    pub fn error(code: &str, message: &str, session_id: Option<String>) -> Self {
        SignalingMessage::Error {
            code: code.to_string(),
            message: message.to_string(),
            session_id,
        }
    }

    /// Create an answer message
    pub fn answer(sdp: String, session_id: String) -> Self {
        SignalingMessage::Answer { sdp, session_id }
    }

    /// Create an ICE candidate message
    pub fn ice_candidate(
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
        session_id: String,
    ) -> Self {
        SignalingMessage::IceCandidate {
            candidate,
            sdp_mid,
            sdp_mline_index,
            session_id,
        }
    }

    /// Create a session ready message
    pub fn ready(session_id: String, video_codec: &str, data_channel: &str) -> Self {
        SignalingMessage::Ready {
            session_id,
            video_codec: video_codec.to_string(),
            data_channel: data_channel.to_string(),
        }
    }

    /// Get the session ID if present
    pub fn session_id(&self) -> Option<&str> {
        match self {
            SignalingMessage::Offer { session_id, .. } => session_id.as_deref(),
            SignalingMessage::Answer { session_id, .. } => Some(session_id),
            SignalingMessage::IceCandidate { session_id, .. } => Some(session_id),
            SignalingMessage::IceComplete { session_id } => Some(session_id),
            SignalingMessage::Ready { session_id, .. } => Some(session_id),
            SignalingMessage::Error { session_id, .. } => session_id.as_deref(),
            SignalingMessage::KeyframeRequest { session_id } => Some(session_id),
            SignalingMessage::BitrateRequest { session_id, .. } => Some(session_id),
            SignalingMessage::Stats { session_id, .. } => Some(session_id),
            SignalingMessage::Close { session_id, .. } => Some(session_id),
            SignalingMessage::Ping { .. } | SignalingMessage::Pong { .. } => None,
        }
    }
}

/// Signaling handler trait for processing messages
pub trait SignalingHandler: Send + Sync {
    /// Handle an incoming signaling message
    fn handle_message(&self, message: SignalingMessage) -> Option<SignalingMessage>;

    /// Handle connection established
    fn on_connected(&self, client_id: &str);

    /// Handle connection closed
    fn on_disconnected(&self, client_id: &str);
}

/// Simple signaling message parser for WebSocket text frames
pub struct SignalingParser;

impl SignalingParser {
    /// Parse a WebSocket text message into a signaling message
    ///
    /// Supports both JSON format and legacy format:
    /// - JSON: `{"type": "offer", "sdp": "..."}`
    /// - Legacy: `webrtc,offer,<sdp>`
    pub fn parse(text: &str) -> Result<SignalingMessage, WebRTCError> {
        let text = text.trim();

        // Try JSON first
        if text.starts_with('{') {
            return SignalingMessage::from_json(text);
        }

        // Try legacy comma-separated format
        if text.starts_with("webrtc,") {
            return Self::parse_legacy(text);
        }

        Err(WebRTCError::SdpError(format!("Unknown message format: {}", &text[..text.len().min(50)])))
    }

    /// Parse legacy comma-separated format
    fn parse_legacy(text: &str) -> Result<SignalingMessage, WebRTCError> {
        let parts: Vec<&str> = text.splitn(4, ',').collect();

        if parts.len() < 2 {
            return Err(WebRTCError::SdpError("Invalid legacy message format".to_string()));
        }

        match parts[1] {
            "offer" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::SdpError("Missing SDP in offer".to_string()));
                }
                Ok(SignalingMessage::Offer {
                    sdp: parts[2].to_string(),
                    session_id: parts.get(3).map(|s| s.to_string()),
                })
            }

            "answer" => {
                if parts.len() < 4 {
                    return Err(WebRTCError::SdpError("Missing SDP or session_id in answer".to_string()));
                }
                Ok(SignalingMessage::Answer {
                    sdp: parts[2].to_string(),
                    session_id: parts[3].to_string(),
                })
            }

            "ice" => {
                if parts.len() < 4 {
                    return Err(WebRTCError::SdpError("Missing ICE candidate data".to_string()));
                }
                Ok(SignalingMessage::IceCandidate {
                    candidate: parts[2].to_string(),
                    sdp_mid: Some("0".to_string()),
                    sdp_mline_index: Some(0),
                    session_id: parts[3].to_string(),
                })
            }

            "keyframe" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::SdpError("Missing session_id in keyframe request".to_string()));
                }
                Ok(SignalingMessage::KeyframeRequest {
                    session_id: parts[2].to_string(),
                })
            }

            "close" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::SdpError("Missing session_id in close".to_string()));
                }
                Ok(SignalingMessage::Close {
                    session_id: parts[2].to_string(),
                    reason: parts.get(3).map(|s| s.to_string()),
                })
            }

            cmd => Err(WebRTCError::SdpError(format!("Unknown legacy command: {}", cmd))),
        }
    }

    /// Format a signaling message for WebSocket transmission
    pub fn format(message: &SignalingMessage) -> Result<String, WebRTCError> {
        // Always use JSON format for outgoing messages
        message.to_json()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_offer() {
        let json = r#"{"type": "offer", "sdp": "v=0\r\n..."}"#;
        let msg = SignalingParser::parse(json).unwrap();
        match msg {
            SignalingMessage::Offer { sdp, .. } => assert!(sdp.starts_with("v=0")),
            _ => panic!("Expected Offer"),
        }
    }

    #[test]
    fn test_parse_legacy_offer() {
        let text = "webrtc,offer,v=0\r\n...";
        let msg = SignalingParser::parse(text).unwrap();
        match msg {
            SignalingMessage::Offer { sdp, .. } => assert!(sdp.starts_with("v=0")),
            _ => panic!("Expected Offer"),
        }
    }

    #[test]
    fn test_message_serialization() {
        let msg = SignalingMessage::answer("v=0...".to_string(), "session123".to_string());
        let json = msg.to_json().unwrap();
        assert!(json.contains("answer"));
        assert!(json.contains("session123"));
    }

    #[test]
    fn test_error_message() {
        let msg = SignalingMessage::error("INVALID_SDP", "SDP parsing failed", Some("sess1".to_string()));
        let json = msg.to_json().unwrap();
        assert!(json.contains("error"));
        assert!(json.contains("INVALID_SDP"));
    }
}
