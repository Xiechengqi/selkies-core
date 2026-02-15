//! WebRTC Session Management (str0m)
//!
//! Manages the lifecycle of WebRTC sessions including:
//! - Session creation via SDP offer/answer
//! - ICE-TCP connection acceptance and routing
//! - Session state tracking and cleanup

use super::rtc_session::{self, RtcSession};
use super::WebRTCError;
use crate::clipboard::ClipboardReceiver;
use crate::config::WebRTCConfig;
use crate::file_upload::{FileUploadHandler, FileUploadSettings};
use crate::input::InputEventData;
use crate::runtime_settings::RuntimeSettings;
use crate::web::SharedState;

use log::{info, warn};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};

use str0m::Input;

/// How long a pending session can wait for a TCP connection before being reaped.
const PENDING_SESSION_TTL: Duration = Duration::from_secs(30);

/// Session manager for handling multiple str0m WebRTC sessions.
///
/// Unlike the old webrtc-rs SessionManager, this one:
/// - Creates str0m Rtc instances (synchronous, Sans-I/O)
/// - Injects TCP passive ICE candidates (same port as HTTP)
/// - Hands off TCP connections to per-session drive loops
/// - Does NOT manage UDP sockets or STUN/TURN
pub struct SessionManager {
    /// Active sessions awaiting TCP connection (after SDP but before ICE-TCP)
    pending_sessions: Arc<RwLock<HashMap<String, PendingSession>>>,
    /// WebRTC configuration
    config: WebRTCConfig,
    /// Input event sender
    input_tx: mpsc::UnboundedSender<InputEventData>,
    /// File upload settings
    upload_settings: FileUploadSettings,
    /// Runtime settings
    runtime_settings: Arc<RuntimeSettings>,
    /// Shared state
    shared_state: Arc<SharedState>,
    /// Maximum concurrent sessions
    max_sessions: usize,
    /// The listen address for TCP passive candidates
    listen_addr: SocketAddr,
}

/// A pending session wraps an RtcSession with a creation timestamp for TTL cleanup.
struct PendingSession {
    session: RtcSession,
    created_at: Instant,
}

impl SessionManager {
    /// Create a new session manager.
    ///
    /// `listen_addr` is the public-facing address to advertise in ICE TCP
    /// passive candidates. This should be the address the browser will
    /// connect to (e.g., the tunnel endpoint's public IP:port).
    pub fn new(
        config: WebRTCConfig,
        input_tx: mpsc::UnboundedSender<InputEventData>,
        upload_settings: FileUploadSettings,
        runtime_settings: Arc<RuntimeSettings>,
        shared_state: Arc<SharedState>,
        max_sessions: usize,
        listen_addr: SocketAddr,
    ) -> Self {
        let mgr = Self {
            pending_sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
            input_tx,
            upload_settings,
            runtime_settings,
            shared_state,
            max_sessions,
            listen_addr,
        };

        // Spawn a background task to reap stale pending sessions
        let pending = mgr.pending_sessions.clone();
        let state = mgr.shared_state.clone();
        tokio::spawn(async move {
            reap_stale_sessions(pending, state).await;
        });

        mgr
    }

    /// Create a new session and process the SDP offer.
    ///
    /// Returns (session_id, answer_sdp).
    /// The session is stored in `pending_sessions` until a TCP connection
    /// arrives and is matched via `accepts()`.
    pub async fn create_session_with_offer(
        &self,
        offer_sdp: &str,
    ) -> Result<(String, String), WebRTCError> {
        let session_id = uuid::Uuid::new_v4().to_string();

        // Create str0m Rtc instance
        let mut session = RtcSession::new(session_id.clone());

        // Add TCP passive candidate pointing to our listen address
        session.add_local_tcp_candidate(self.listen_addr)?;
        info!("Session {} added TCP candidate: {}", session_id, self.listen_addr);

        // Accept the SDP offer and generate answer
        let answer_sdp = session.accept_offer(offer_sdp)?;
        info!("Session {} SDP answer generated", session_id);

        // Check capacity and insert under a single write lock to avoid TOCTOU race
        let mut pending = self.pending_sessions.write().await;
        if pending.len() >= self.max_sessions {
            return Err(WebRTCError::ConnectionFailed("Maximum sessions reached".to_string()));
        }
        pending.insert(session_id.clone(), PendingSession {
            session,
            created_at: Instant::now(),
        });
        self.shared_state.increment_webrtc_sessions();

        Ok((session_id, answer_sdp))
    }

    /// Remove a pending session by ID (e.g., when signaling WebSocket closes).
    ///
    /// Returns true if a session was removed.
    pub async fn remove_pending_session(&self, session_id: &str) -> bool {
        let mut pending = self.pending_sessions.write().await;
        if pending.remove(session_id).is_some() {
            self.shared_state.decrement_webrtc_sessions();
            info!("Removed pending session: {}", session_id);
            true
        } else {
            false
        }
    }

    /// Try to match an incoming TCP connection to a pending session.
    ///
    /// Called by the TCP protocol splitter when it detects a STUN/DTLS
    /// first byte on an accepted connection. The first packet is used
    /// to identify which Rtc instance should handle this connection
    /// via `rtc.accepts()`.
    ///
    /// If matched, the session is removed from pending and spawned
    /// as a drive loop task.
    pub async fn handle_ice_tcp_connection(
        &self,
        tcp_stream: TcpStream,
        peer_addr: SocketAddr,
        local_addr: SocketAddr,
        first_packet: &[u8],
    ) -> Result<(), WebRTCError> {
        let mut pending = self.pending_sessions.write().await;

        // Find the session that accepts this packet
        let mut matched_id = None;
        for (id, ps) in pending.iter() {
            let recv = str0m::net::Receive {
                proto: str0m::net::Protocol::Tcp,
                source: peer_addr,
                destination: local_addr,
                contents: match first_packet.try_into() {
                    Ok(c) => c,
                    Err(_) => continue,
                },
            };
            let input = Input::Receive(std::time::Instant::now(), recv);
            if ps.session.rtc.accepts(&input) {
                matched_id = Some(id.clone());
                break;
            }
        }

        let session_id = matched_id.ok_or_else(|| {
            WebRTCError::SessionNotFound("No session accepts this TCP connection".to_string())
        })?;

        let mut session = pending.remove(&session_id).unwrap().session;
        drop(pending);

        info!("Session {} matched TCP connection from {}", session_id, peer_addr);

        // Feed the first packet into str0m
        let recv = str0m::net::Receive {
            proto: str0m::net::Protocol::Tcp,
            source: peer_addr,
            destination: local_addr,
            contents: first_packet.try_into()
                .map_err(|e| WebRTCError::ConnectionFailed(format!("First packet parse: {}", e)))?,
        };
        session.rtc.handle_input(Input::Receive(std::time::Instant::now(), recv))
            .map_err(|e| WebRTCError::ConnectionFailed(format!("handle_input: {}", e)))?;

        // Spawn the session drive loop
        let shared_state = self.shared_state.clone();
        let input_tx = self.input_tx.clone();
        let upload_handler = Arc::new(Mutex::new(
            FileUploadHandler::new(self.upload_settings.clone())
        ));
        let clipboard = Arc::new(Mutex::new(
            ClipboardReceiver::new(self.shared_state.clone())
        ));
        let runtime_settings = self.runtime_settings.clone();

        tokio::spawn(async move {
            rtc_session::drive_session(
                session,
                tcp_stream,
                peer_addr,
                local_addr,
                shared_state,
                input_tx,
                upload_handler,
                clipboard,
                runtime_settings,
            ).await;
        });

        Ok(())
    }

    /// Get WebRTC configuration
    pub fn config(&self) -> &WebRTCConfig {
        &self.config
    }
}

/// Background task that periodically removes pending sessions that have
/// exceeded their TTL without receiving a TCP connection.
async fn reap_stale_sessions(
    pending: Arc<RwLock<HashMap<String, PendingSession>>>,
    shared_state: Arc<SharedState>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        interval.tick().await;
        let mut map = pending.write().await;
        let now = Instant::now();
        let stale: Vec<String> = map.iter()
            .filter(|(_, ps)| now.duration_since(ps.created_at) > PENDING_SESSION_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            map.remove(id);
            shared_state.decrement_webrtc_sessions();
            warn!("Reaped stale pending session: {}", id);
        }
    }
}
