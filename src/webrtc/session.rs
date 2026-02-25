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

use super::tcp_framing::frame_packet;
use log::{info, warn, debug};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};

use str0m::{Input, Output};

/// How long a pending session can wait for a TCP connection before being reaped.
const PENDING_SESSION_TTL: Duration = Duration::from_secs(30);

/// Session manager for handling multiple str0m WebRTC sessions.
///
/// Unlike the old webrtc-rs SessionManager, this one:
/// - Creates str0m Rtc instances (synchronous, Sans-I/O)
/// - Injects TCP passive ICE candidates (same port as HTTP)
/// - Hands off TCP connections to per-session drive loops
/// - Does NOT manage external relay servers
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
    candidate_addr: SocketAddr,
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
        client_host: Option<&str>,
    ) -> Result<(String, String), WebRTCError> {
        let session_id = uuid::Uuid::new_v4().to_string();

        // Create str0m Rtc instance
        let mut session = RtcSession::new(session_id.clone());

        // Determine the ICE candidate address.
        // If the browser connected via a tunnel/proxy, use the Host header
        // so the ICE-TCP candidate points to the same public address.
        let candidate_addr = resolve_candidate_addr(&self.config, client_host, self.listen_addr);

        // Add TCP passive candidate
        session.add_local_tcp_candidate(candidate_addr)?;
        info!("Session {} added TCP candidate: {} (host header: {:?})", session_id, candidate_addr, client_host);

        // Accept the SDP offer and generate answer
        info!("Session {} SDP offer ({} bytes): {:?}", session_id, offer_sdp.len(), &offer_sdp[..offer_sdp.len().min(200)]);
        let answer_sdp = session.accept_offer(offer_sdp)?;
        info!("Session {} SDP answer generated ({} bytes):\n{}", session_id, answer_sdp.len(), answer_sdp);

        // Check capacity and insert under a single write lock to avoid TOCTOU race
        let mut pending = self.pending_sessions.write().await;
        if pending.len() >= self.max_sessions {
            return Err(WebRTCError::ConnectionFailed("Maximum sessions reached".to_string()));
        }
        pending.insert(session_id.clone(), PendingSession {
            session,
            candidate_addr,
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
    /// Called by the TCP protocol splitter when it detects ICE/DTLS
    /// first byte on an accepted connection. The first packet is used
    /// to identify which Rtc instance should handle this connection
    /// via `rtc.accepts()`.
    ///
    /// If matched, the session is removed from pending and spawned
    /// as a drive loop task.
    pub async fn handle_ice_tcp_connection(
        &self,
        mut tcp_stream: TcpStream,
        peer_addr: SocketAddr,
        _local_addr: SocketAddr,
        first_packet: &[u8],
    ) -> Result<(), WebRTCError> {
        // Decode RFC 4571 framing — the raw TCP data has a 2-byte length prefix
        let mut decoder = super::tcp_framing::TcpFrameDecoder::new();
        decoder.extend(first_packet);
        let ice_pkt = match decoder.next_packet() {
            Ok(Some(pkt)) => pkt,
            Ok(None) => {
                return Err(WebRTCError::ConnectionFailed(
                    "Incomplete RFC 4571 frame in first packet".to_string(),
                ));
            }
            Err(e) => {
                return Err(WebRTCError::ConnectionFailed(format!(
                    "Invalid RFC 4571 frame in first packet: {:?}",
                    e
                )));
            }
        };

        let mut pending = self.pending_sessions.write().await;

        // Find the session that accepts this deframed ICE packet
        let mut matched_id = None;
        for (id, ps) in pending.iter() {
            let recv = str0m::net::Receive {
                proto: str0m::net::Protocol::Tcp,
                source: peer_addr,
                destination: ps.candidate_addr,
                contents: match (&*ice_pkt).try_into() {
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

        let ps = pending.remove(&session_id).unwrap();
        let mut session = ps.session;
        let candidate_addr = ps.candidate_addr;
        drop(pending);

        info!("Session {} matched TCP connection from {}", session_id, peer_addr);

        // Feed the first deframed packet into str0m
        let recv = str0m::net::Receive {
            proto: str0m::net::Protocol::Tcp,
            source: peer_addr,
            destination: candidate_addr,
            contents: (&*ice_pkt).try_into()
                .map_err(|e| WebRTCError::ConnectionFailed(format!("First packet parse: {}", e)))?,
        };
        session.rtc.handle_input(Input::Receive(std::time::Instant::now(), recv))
            .map_err(|e| WebRTCError::ConnectionFailed(format!("handle_input: {}", e)))?;

        // Immediately drain str0m outputs (DTLS handshake responses, ICE checks)
        // BEFORE spawning the drive loop. Without this, the browser's DTLS
        // handshake times out behind TCP proxies because responses sit queued
        // in str0m until the tokio task is scheduled.
        drain_initial_outputs(&mut session, &mut tcp_stream).await?;

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
                candidate_addr,
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

    /// Get the ICE-TCP candidate listen address.
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen_addr
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

/// Drain str0m outputs immediately after feeding the first ICE packet.
/// This ensures DTLS handshake responses are sent back to the browser
/// before the drive loop task is scheduled, preventing timeout behind proxies.
async fn drain_initial_outputs(
    session: &mut RtcSession,
    tcp_stream: &mut TcpStream,
) -> Result<(), WebRTCError> {
    let mut count = 0u32;
    loop {
        match session.rtc.poll_output() {
            Ok(Output::Transmit(t)) => {
                let framed = frame_packet(&t.contents);
                tcp_stream.write_all(&framed).await
                    .map_err(|e| WebRTCError::ConnectionFailed(
                        format!("Initial drain TCP write: {}", e),
                    ))?;
                count += 1;
            }
            Ok(Output::Event(event)) => {
                debug!("Session {} initial event: {:?}", session.id, event);
            }
            Ok(Output::Timeout(_)) => break,
            Err(e) => {
                return Err(WebRTCError::ConnectionFailed(
                    format!("Initial drain poll_output: {}", e),
                ));
            }
        }
    }
    if count > 0 {
        tcp_stream.flush().await.ok();
        info!("Session {} drained {} initial packets", session.id, count);
    }
    Ok(())
}

/// Parse a Host header value (e.g. "example.com:8008" or "1.2.3.4:8008")
/// into a SocketAddr. Falls back to `default_port` if no port is specified.
fn parse_host_to_addr(host: &str, default_port: u16) -> Option<SocketAddr> {
    // Try direct parse first (covers "1.2.3.4:8008")
    if let Ok(addr) = host.parse::<SocketAddr>() {
        return Some(addr);
    }
    // Try as "host:port" where host is a domain or bare IP
    if let Some((h, p)) = host.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            if let Ok(ip) = h.parse::<std::net::IpAddr>() {
                return Some(SocketAddr::new(ip, port));
            }
            // Domain name — resolve it
            use std::net::ToSocketAddrs;
            return format!("{}:{}", h, port).to_socket_addrs().ok()?.next();
        }
    }
    // Bare host without port
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Some(SocketAddr::new(ip, default_port));
    }
    use std::net::ToSocketAddrs;
    format!("{}:{}", host, default_port).to_socket_addrs().ok()?.next()
}

fn resolve_candidate_addr(
    config: &WebRTCConfig,
    client_host: Option<&str>,
    listen_addr: SocketAddr,
) -> SocketAddr {
    if let Some(ref public_candidate) = config.public_candidate {
        match public_candidate.parse::<SocketAddr>() {
            Ok(addr) => return addr,
            Err(e) => {
                warn!(
                    "Invalid public_candidate '{}': {} (falling back to other sources)",
                    public_candidate, e
                );
            }
        }
    }

    if config.candidate_from_host_header {
        if let Some(host) = client_host {
            if let Some(addr) = parse_host_to_addr(host, listen_addr.port()) {
                return addr;
            }
        }
    }

    listen_addr
}
