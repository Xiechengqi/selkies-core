//! WebRTC Session Management
//!
//! Manages the lifecycle of WebRTC sessions including:
//! - Session creation and teardown
//! - Video track management

#![allow(dead_code)]
//! - DataChannel handling
//! - Session state tracking

use super::{WebRTCError, peer_connection::PeerConnectionManager, data_channel::InputDataChannel, media_track::VideoTrackWriter};
use crate::config::{WebRTCConfig, VideoCodec};
use crate::input::InputEventData;
use log::{info, debug};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::data_channel::RTCDataChannel;

/// Session state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Session created, awaiting offer/answer
    New,
    /// Connecting (ICE in progress)
    Connecting,
    /// Connected and streaming
    Connected,
    /// Disconnected (can reconnect)
    Disconnected,
    /// Failed (cannot recover)
    Failed,
    /// Closed (intentionally terminated)
    Closed,
}

impl From<RTCPeerConnectionState> for SessionState {
    fn from(state: RTCPeerConnectionState) -> Self {
        match state {
            RTCPeerConnectionState::New => SessionState::New,
            RTCPeerConnectionState::Connecting => SessionState::Connecting,
            RTCPeerConnectionState::Connected => SessionState::Connected,
            RTCPeerConnectionState::Disconnected => SessionState::Disconnected,
            RTCPeerConnectionState::Failed => SessionState::Failed,
            RTCPeerConnectionState::Closed => SessionState::Closed,
            _ => SessionState::New,
        }
    }
}

/// A single WebRTC streaming session
pub struct WebRTCSession {
    /// Unique session ID
    pub id: String,
    /// Peer connection
    pub peer_connection: Arc<RTCPeerConnection>,
    /// Video track for streaming
    pub video_track: Arc<TrackLocalStaticRTP>,
    /// Video track writer
    pub video_writer: Arc<VideoTrackWriter>,
    /// Input data channel (created after connection)
    pub input_channel: Arc<RwLock<Option<Arc<RTCDataChannel>>>>,
    /// Current session state
    pub state: Arc<RwLock<SessionState>>,
    /// Session creation time
    pub created_at: Instant,
    /// Last activity time
    pub last_activity: Arc<RwLock<Instant>>,
    /// Video codec being used
    pub video_codec: VideoCodec,
    /// Client address/identifier
    pub client_id: Option<String>,
}

impl WebRTCSession {
    /// Create a new session
    pub async fn new(
        id: String,
        peer_connection: Arc<RTCPeerConnection>,
        video_track: Arc<TrackLocalStaticRTP>,
        video_codec: VideoCodec,
    ) -> Self {
        let video_writer = Arc::new(VideoTrackWriter::new(video_track.clone(), video_codec));

        Self {
            id,
            peer_connection,
            video_track,
            video_writer,
            input_channel: Arc::new(RwLock::new(None)),
            state: Arc::new(RwLock::new(SessionState::New)),
            created_at: Instant::now(),
            last_activity: Arc::new(RwLock::new(Instant::now())),
            video_codec,
            client_id: None,
        }
    }

    /// Update session state
    pub async fn set_state(&self, state: SessionState) {
        let mut current = self.state.write().await;
        if *current != state {
            debug!("Session {} state change: {:?} -> {:?}", self.id, *current, state);
            *current = state;
        }
    }

    /// Get current state
    pub async fn get_state(&self) -> SessionState {
        *self.state.read().await
    }

    /// Set the input data channel
    pub async fn set_input_channel(&self, channel: Arc<RTCDataChannel>) {
        let mut input = self.input_channel.write().await;
        *input = Some(channel);
    }

    /// Update last activity time
    pub async fn touch(&self) {
        let mut last = self.last_activity.write().await;
        *last = Instant::now();
    }

    /// Get session age
    pub fn age(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }

    /// Get time since last activity
    pub async fn idle_time(&self) -> std::time::Duration {
        self.last_activity.read().await.elapsed()
    }

    /// Write an RTP packet to the video track
    pub async fn write_rtp(&self, packet: &[u8]) -> Result<(), WebRTCError> {
        self.video_writer.write_rtp(packet).await
    }

    /// Send a message through the input channel
    pub async fn send_to_client(&self, message: &str) -> Result<(), WebRTCError> {
        let channel = self.input_channel.read().await;
        if let Some(ref ch) = *channel {
            ch.send(&bytes::Bytes::copy_from_slice(message.as_bytes())).await
                .map_err(|e| WebRTCError::DataChannelError(format!("Send failed: {}", e)))?;
            Ok(())
        } else {
            Err(WebRTCError::DataChannelError("Input channel not ready".to_string()))
        }
    }

    /// Close the session
    pub async fn close(&self) -> Result<(), WebRTCError> {
        self.set_state(SessionState::Closed).await;

        // Close data channel
        if let Some(ref channel) = *self.input_channel.read().await {
            let _ = channel.close().await;
        }

        // Close peer connection
        self.peer_connection.close().await
            .map_err(|e| WebRTCError::ConnectionFailed(format!("Close failed: {}", e)))?;

        info!("Session {} closed", self.id);
        Ok(())
    }
}

/// Session manager for handling multiple WebRTC sessions
pub struct SessionManager {
    /// Active sessions
    sessions: Arc<RwLock<HashMap<String, Arc<WebRTCSession>>>>,
    /// WebRTC configuration
    config: WebRTCConfig,
    /// Peer connection manager
    pc_manager: PeerConnectionManager,
    /// Input event sender
    input_tx: mpsc::UnboundedSender<InputEventData>,
    /// Maximum concurrent sessions
    max_sessions: usize,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(
        config: WebRTCConfig,
        input_tx: mpsc::UnboundedSender<InputEventData>,
        max_sessions: usize,
    ) -> Self {
        let pc_manager = PeerConnectionManager::new(config.clone());

        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
            pc_manager,
            input_tx,
            max_sessions,
        }
    }

    /// Create a new session
    pub async fn create_session(&self) -> Result<Arc<WebRTCSession>, WebRTCError> {
        // Check session limit
        let sessions = self.sessions.read().await;
        if sessions.len() >= self.max_sessions {
            return Err(WebRTCError::ConnectionFailed("Maximum sessions reached".to_string()));
        }
        drop(sessions);

        // Generate session ID
        let session_id = uuid::Uuid::new_v4().to_string();

        // Create peer connection
        let peer_connection = self.pc_manager.create_peer_connection().await?;

        // Create video track
        let video_track = self.pc_manager.create_video_track(self.config.video_codec)?;

        // Add track to peer connection
        peer_connection.add_track(video_track.clone()).await
            .map_err(|e| WebRTCError::MediaError(format!("Failed to add video track: {}", e)))?;

        // Create session
        let session = Arc::new(WebRTCSession::new(
            session_id.clone(),
            peer_connection.clone(),
            video_track,
            self.config.video_codec,
        ).await);

        // Set up callbacks
        self.setup_session_callbacks(&session).await;

        // Store session
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), session.clone());

        info!("Created WebRTC session: {}", session_id);
        Ok(session)
    }

    /// Set up session callbacks
    async fn setup_session_callbacks(&self, session: &Arc<WebRTCSession>) {
        let _session_weak = Arc::downgrade(session);
        let sessions = self.sessions.clone();
        let session_id = session.id.clone();

        // Connection state change callback
        let session_id_clone = session_id.clone();
        session.peer_connection.on_peer_connection_state_change(Box::new(move |state| {
            let session_id = session_id_clone.clone();
            let sessions = sessions.clone();

            Box::pin(async move {
                info!("Session {} connection state: {:?}", session_id, state);

                let sessions_read = sessions.read().await;
                if let Some(session) = sessions_read.get(&session_id) {
                    session.set_state(SessionState::from(state)).await;

                    // Clean up on failure/close
                    if state == RTCPeerConnectionState::Failed || state == RTCPeerConnectionState::Closed {
                        drop(sessions_read);
                        let mut sessions_write = sessions.write().await;
                        sessions_write.remove(&session_id);
                        info!("Removed session {} due to state {:?}", session_id, state);
                    }
                }
            })
        }));

        // Data channel callback
        let input_tx = self.input_tx.clone();
        let session_for_dc = session.clone();
        session.peer_connection.on_data_channel(Box::new(move |channel| {
            let input_tx = input_tx.clone();
            let session = session_for_dc.clone();

            Box::pin(async move {
                let label = channel.label().to_string();
                info!("Data channel opened: {}", label);

                if label == "input" || label.starts_with("input") {
                    session.set_input_channel(channel.clone()).await;

                    // Set up input handler
                    let input_handler = InputDataChannel::new(channel, input_tx);
                    input_handler.setup_handlers().await;
                }
            })
        }));
    }

    /// Get a session by ID
    pub async fn get_session(&self, session_id: &str) -> Option<Arc<WebRTCSession>> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    /// Remove a session
    pub async fn remove_session(&self, session_id: &str) -> Option<Arc<WebRTCSession>> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.remove(session_id);
        if let Some(ref s) = session {
            let _ = s.close().await;
        }
        session
    }

    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<Arc<WebRTCSession>> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    /// Get number of active sessions
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    /// Broadcast RTP packet to all connected sessions
    pub async fn broadcast_rtp(&self, packet: &[u8]) {
        let sessions = self.sessions.read().await;
        for session in sessions.values() {
            if session.get_state().await == SessionState::Connected {
                let _ = session.write_rtp(packet).await;
            }
        }
    }

    /// Clean up stale sessions
    pub async fn cleanup_stale_sessions(&self, timeout_secs: u64) {
        let mut to_remove = Vec::new();

        {
            let sessions = self.sessions.read().await;
            for (id, session) in sessions.iter() {
                let state = session.get_state().await;
                let idle = session.idle_time().await;

                if state == SessionState::Failed
                    || state == SessionState::Closed
                    || (state == SessionState::Disconnected && idle.as_secs() > timeout_secs)
                {
                    to_remove.push(id.clone());
                }
            }
        }

        for id in to_remove {
            self.remove_session(&id).await;
            info!("Cleaned up stale session: {}", id);
        }
    }

    /// Handle SDP offer from client
    pub async fn handle_offer(&self, session_id: &str, sdp: &str) -> Result<String, WebRTCError> {
        let session = self.get_session(session_id).await
            .ok_or_else(|| WebRTCError::SessionNotFound(session_id.to_string()))?;

        let answer_sdp = PeerConnectionManager::handle_offer(&session.peer_connection, sdp).await?;
        session.touch().await;

        Ok(answer_sdp)
    }

    /// Add ICE candidate from client
    pub async fn add_ice_candidate(
        &self,
        session_id: &str,
        candidate: &str,
        sdp_mid: Option<&str>,
        sdp_mline_index: Option<u16>,
    ) -> Result<(), WebRTCError> {
        let session = self.get_session(session_id).await
            .ok_or_else(|| WebRTCError::SessionNotFound(session_id.to_string()))?;

        PeerConnectionManager::add_ice_candidate(
            &session.peer_connection,
            candidate,
            sdp_mid,
            sdp_mline_index,
        ).await?;

        session.touch().await;
        Ok(())
    }

    /// Get WebRTC configuration
    pub fn config(&self) -> &WebRTCConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_state_from_rtc_state() {
        assert_eq!(SessionState::from(RTCPeerConnectionState::New), SessionState::New);
        assert_eq!(SessionState::from(RTCPeerConnectionState::Connected), SessionState::Connected);
        assert_eq!(SessionState::from(RTCPeerConnectionState::Failed), SessionState::Failed);
    }
}
