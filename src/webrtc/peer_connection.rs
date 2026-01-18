//! WebRTC PeerConnection management
//!
//! Handles the creation and lifecycle of RTCPeerConnection instances.

#![allow(dead_code)]

use super::WebRTCError;
use crate::config::{WebRTCConfig, VideoCodec};
use std::sync::Arc;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264, MIME_TYPE_VP8, MIME_TYPE_VP9};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::data_channel::RTCDataChannel;

/// Callback for ICE candidate generation
pub type IceCandidateCallback = Box<dyn Fn(String) + Send + Sync>;

/// Callback for connection state changes
pub type StateChangeCallback = Box<dyn Fn(RTCPeerConnectionState) + Send + Sync>;

/// Callback for data channel messages
pub type DataChannelCallback = Box<dyn Fn(Vec<u8>) + Send + Sync>;

/// PeerConnection manager for WebRTC sessions
pub struct PeerConnectionManager {
    config: WebRTCConfig,
}

impl PeerConnectionManager {
    /// Create a new PeerConnection manager
    pub fn new(config: WebRTCConfig) -> Self {
        Self { config }
    }

    /// Create a new PeerConnection with the configured settings
    pub async fn create_peer_connection(&self) -> Result<Arc<RTCPeerConnection>, WebRTCError> {
        // Create media engine with codec support
        let mut media_engine = MediaEngine::default();

        // Register video codecs based on configuration
        self.register_video_codecs(&mut media_engine)?;

        // Create interceptor registry for RTCP feedback
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)
            .map_err(|e| WebRTCError::ConnectionFailed(format!("Failed to register interceptors: {}", e)))?;

        // Build API
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        // Convert ICE servers configuration
        let ice_servers = self.config.ice_servers.iter().map(|server| {
            RTCIceServer {
                urls: server.urls.clone(),
                username: server.username.clone().unwrap_or_default(),
                credential: server.credential.clone().unwrap_or_default(),
                ..Default::default()
            }
        }).collect();

        let rtc_config = RTCConfiguration {
            ice_servers,
            ..Default::default()
        };

        // Create peer connection
        let peer_connection = api.new_peer_connection(rtc_config).await
            .map_err(|e| WebRTCError::ConnectionFailed(format!("Failed to create peer connection: {}", e)))?;

        Ok(Arc::new(peer_connection))
    }

    /// Register video codecs in the media engine
    fn register_video_codecs(&self, media_engine: &mut MediaEngine) -> Result<(), WebRTCError> {
        // Register H.264
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_string(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_string(),
                    rtcp_feedback: vec![],
                },
                payload_type: 96,
                ..Default::default()
            },
            RTPCodecType::Video,
        ).map_err(|e| WebRTCError::ConnectionFailed(format!("Failed to register H264: {}", e)))?;

        // Register VP8
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_VP8.to_string(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line: "".to_string(),
                    rtcp_feedback: vec![],
                },
                payload_type: 97,
                ..Default::default()
            },
            RTPCodecType::Video,
        ).map_err(|e| WebRTCError::ConnectionFailed(format!("Failed to register VP8: {}", e)))?;

        // Register VP9
        media_engine.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_VP9.to_string(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line: "profile-id=0".to_string(),
                    rtcp_feedback: vec![],
                },
                payload_type: 98,
                ..Default::default()
            },
            RTPCodecType::Video,
        ).map_err(|e| WebRTCError::ConnectionFailed(format!("Failed to register VP9: {}", e)))?;

        Ok(())
    }

    /// Create a video track for the specified codec
    pub fn create_video_track(&self, codec: VideoCodec) -> Result<Arc<TrackLocalStaticRTP>, WebRTCError> {
        let (mime_type, _payload_type) = match codec {
            VideoCodec::H264 => (MIME_TYPE_H264, 96),
            VideoCodec::VP8 => (MIME_TYPE_VP8, 97),
            VideoCodec::VP9 => (MIME_TYPE_VP9, 98),
            VideoCodec::AV1 => ("video/AV1", 99),
        };

        let track = TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: mime_type.to_string(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "".to_string(),
                rtcp_feedback: vec![],
            },
            format!("video-{}", uuid::Uuid::new_v4()),
            "selkies-stream".to_string(),
        );

        Ok(Arc::new(track))
    }

    /// Set up connection state change callback
    pub async fn setup_state_callback(
        peer_connection: &Arc<RTCPeerConnection>,
        callback: impl Fn(RTCPeerConnectionState) + Send + Sync + 'static,
    ) {
        let callback = Arc::new(callback);
        peer_connection.on_peer_connection_state_change(Box::new(move |state| {
            let callback = callback.clone();
            Box::pin(async move {
                callback(state);
            })
        }));
    }

    /// Set up ICE candidate callback
    pub async fn setup_ice_callback(
        peer_connection: &Arc<RTCPeerConnection>,
        callback: impl Fn(Option<String>) + Send + Sync + 'static,
    ) {
        let callback = Arc::new(callback);
        peer_connection.on_ice_candidate(Box::new(move |candidate| {
            let callback = callback.clone();
            Box::pin(async move {
                let candidate_str = candidate.map(|c| c.to_json().map(|j| j.candidate).unwrap_or_default());
                callback(candidate_str);
            })
        }));
    }

    /// Create an SDP offer
    pub async fn create_offer(peer_connection: &Arc<RTCPeerConnection>) -> Result<String, WebRTCError> {
        let offer = peer_connection.create_offer(None).await
            .map_err(|e| WebRTCError::SdpError(format!("Failed to create offer: {}", e)))?;

        peer_connection.set_local_description(offer.clone()).await
            .map_err(|e| WebRTCError::SdpError(format!("Failed to set local description: {}", e)))?;

        Ok(offer.sdp)
    }

    /// Handle an SDP answer
    pub async fn handle_answer(
        peer_connection: &Arc<RTCPeerConnection>,
        sdp: &str,
    ) -> Result<(), WebRTCError> {
        let answer = RTCSessionDescription::answer(sdp.to_string())
            .map_err(|e| WebRTCError::SdpError(format!("Invalid SDP answer: {}", e)))?;

        peer_connection.set_remote_description(answer).await
            .map_err(|e| WebRTCError::SdpError(format!("Failed to set remote description: {}", e)))?;

        Ok(())
    }

    /// Handle an SDP offer (for answering)
    pub async fn handle_offer(
        peer_connection: &Arc<RTCPeerConnection>,
        sdp: &str,
    ) -> Result<String, WebRTCError> {
        let offer = RTCSessionDescription::offer(sdp.to_string())
            .map_err(|e| WebRTCError::SdpError(format!("Invalid SDP offer: {}", e)))?;

        peer_connection.set_remote_description(offer).await
            .map_err(|e| WebRTCError::SdpError(format!("Failed to set remote description: {}", e)))?;

        let answer = peer_connection.create_answer(None).await
            .map_err(|e| WebRTCError::SdpError(format!("Failed to create answer: {}", e)))?;

        peer_connection.set_local_description(answer.clone()).await
            .map_err(|e| WebRTCError::SdpError(format!("Failed to set local description: {}", e)))?;

        Ok(answer.sdp)
    }

    /// Add an ICE candidate
    pub async fn add_ice_candidate(
        peer_connection: &Arc<RTCPeerConnection>,
        candidate: &str,
        sdp_mid: Option<&str>,
        sdp_mline_index: Option<u16>,
    ) -> Result<(), WebRTCError> {
        use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

        let candidate_init = RTCIceCandidateInit {
            candidate: candidate.to_string(),
            sdp_mid: sdp_mid.map(|s| s.to_string()),
            sdp_mline_index,
            username_fragment: None,
        };

        peer_connection.add_ice_candidate(candidate_init).await
            .map_err(|e| WebRTCError::IceError(format!("Failed to add ICE candidate: {}", e)))?;

        Ok(())
    }

    /// Create a data channel for input
    pub async fn create_data_channel(
        peer_connection: &Arc<RTCPeerConnection>,
        label: &str,
    ) -> Result<Arc<RTCDataChannel>, WebRTCError> {
        use webrtc::data_channel::data_channel_init::RTCDataChannelInit;

        let config = RTCDataChannelInit {
            ordered: Some(true),
            max_retransmits: Some(0),  // Unreliable for low latency
            ..Default::default()
        };

        let channel = peer_connection.create_data_channel(label, Some(config)).await
            .map_err(|e| WebRTCError::DataChannelError(format!("Failed to create data channel: {}", e)))?;

        Ok(channel)
    }

    /// Close a peer connection
    pub async fn close(peer_connection: &Arc<RTCPeerConnection>) -> Result<(), WebRTCError> {
        peer_connection.close().await
            .map_err(|e| WebRTCError::ConnectionFailed(format!("Failed to close connection: {}", e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebRTCConfig;

    #[tokio::test]
    async fn test_peer_connection_manager_creation() {
        let config = WebRTCConfig::default();
        let manager = PeerConnectionManager::new(config);
        // Manager should be created successfully
        assert!(manager.config.enabled);
    }
}
