//! str0m Sans-I/O WebRTC session driver
//!
//! Each RtcSession wraps a str0m `Rtc` instance and drives it via
//! a tokio task that multiplexes TCP I/O, RTP broadcast, audio, and
//! text forwarding through a single event loop.

use super::tcp_framing::{frame_packet, TcpFrameDecoder};
use super::data_channel::InputDataChannel;
use super::media_track::rtp_util;
use super::WebRTCError;
use crate::clipboard::ClipboardReceiver;
use crate::file_upload::FileUploadHandler;
use crate::input::{InputEvent, InputEventData};
use crate::runtime_settings::RuntimeSettings;
use crate::web::SharedState;

use log::{debug, error, info, warn};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};

use str0m::channel::{ChannelData, ChannelId};
use str0m::media::{MediaKind, Mid, Pt};
use str0m::net::{self, Protocol};
use str0m::rtp::SeqNo;
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use str0m::change::SdpOffer;

/// A single str0m WebRTC session bound to a TCP connection.
pub struct RtcSession {
    /// Unique session ID
    pub id: String,
    /// The str0m Sans-I/O instance
    pub rtc: Rtc,
    /// Video media line ID (set after SDP negotiation)
    pub video_mid: Option<Mid>,
    /// Audio media line ID (set after SDP negotiation)
    pub audio_mid: Option<Mid>,
    /// DataChannel ID for input
    pub dc_id: Option<ChannelId>,
    /// Negotiated video payload type (discovered from SDP, e.g. H264 PT)
    video_pt: Option<Pt>,
    /// Negotiated audio payload type (discovered from SDP)
    audio_pt: Option<Pt>,
    /// Whether the session is connected
    pub connected: bool,
    /// RTP sequence counter for video (str0m RTP mode needs us to supply seq)
    video_seq: u64,
    /// RTP sequence counter for audio
    audio_seq: u64,
}

impl RtcSession {
    /// Create a new RtcSession with str0m configured for ICE-lite + RTP mode.
    pub fn new(id: String) -> Self {
        let now = Instant::now();
        let rtc = Rtc::builder()
            .set_ice_lite(true)
            .set_rtp_mode(true)
            .build(now);

        Self {
            id,
            rtc,
            video_mid: None,
            audio_mid: None,
            dc_id: None,
            video_pt: None,
            audio_pt: None,
            connected: false,
            video_seq: 0,
            audio_seq: 0,
        }
    }

    /// Add a TCP passive ICE candidate for the given listen address.
    pub fn add_local_tcp_candidate(&mut self, addr: SocketAddr) -> Result<(), WebRTCError> {
        use str0m::net::TcpType;
        let candidate = Candidate::builder()
            .tcp()
            .host(addr)
            .tcptype(TcpType::Passive)
            .build()
            .map_err(|e| WebRTCError::IceError(format!("Failed to build TCP candidate: {}", e)))?;
        self.rtc.add_local_candidate(candidate);
        Ok(())
    }

    /// Accept an SDP offer and return the SDP answer string.
    pub fn accept_offer(&mut self, offer_sdp: &str) -> Result<String, WebRTCError> {
        let offer = SdpOffer::from_sdp_string(offer_sdp)
            .map_err(|e| WebRTCError::SdpError(format!("Failed to parse SDP offer: {}", e)))?;

        let answer = self.rtc.sdp_api().accept_offer(offer)
            .map_err(|e| WebRTCError::SdpError(format!("Failed to accept offer: {}", e)))?;

        // Discover media line IDs from the SDP negotiation
        Ok(answer.to_sdp_string())
    }

    /// Write a video RTP packet from GStreamer into str0m.
    pub fn write_video_rtp(&mut self, rtp_data: &[u8]) -> Result<(), WebRTCError> {
        let mid = match self.video_mid {
            Some(mid) => mid,
            None => return Ok(()), // Not yet negotiated
        };

        if rtp_data.len() < 12 {
            return Ok(()); // Too small
        }

        // Use the negotiated PT from SDP, not the GStreamer-assigned one.
        // GStreamer uses PT 96 by default for H.264, but str0m may have
        // mapped PT 96 to VP8 — we must use the actual H.264 PT from SDP.
        let pt = match self.video_pt {
            Some(pt) => pt,
            None => Pt::new_with_value(rtp_util::get_payload_type(rtp_data).unwrap_or(96)),
        };
        let marker = rtp_util::is_marker_set(rtp_data);
        let timestamp = rtp_util::get_timestamp(rtp_data).unwrap_or(0);
        let header_len = rtp_util::header_length(rtp_data).unwrap_or(12);
        let payload = if rtp_data.len() > header_len {
            rtp_data[header_len..].to_vec()
        } else {
            return Ok(());
        };

        let seq = SeqNo::from(self.video_seq);
        self.video_seq += 1;

        if let Some(stream_tx) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
            let _ = stream_tx.write_rtp(
                pt,
                seq,
                timestamp,
                Instant::now(),
                marker,
                str0m::rtp::ExtensionValues::default(),
                true, // nackable
                payload,
            );
            if self.video_seq <= 5 {
                info!("Session {} write_rtp seq={} pt={:?} ts={} marker={} len={}",
                    self.id, self.video_seq - 1, pt, timestamp, marker, rtp_data.len());
            }
        }

        Ok(())
    }

    /// Write an audio RTP packet (Opus) into str0m.
    pub fn write_audio_rtp(&mut self, opus_data: &[u8], timestamp: u32) -> Result<(), WebRTCError> {
        let mid = match self.audio_mid {
            Some(mid) => mid,
            None => return Ok(()),
        };

        let seq_no = SeqNo::from(self.audio_seq);
        self.audio_seq += 1;

        if let Some(stream_tx) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
            let _ = stream_tx.write_rtp(
                self.audio_pt.unwrap_or(Pt::new_with_value(111)),
                seq_no,
                timestamp,
                Instant::now(),
                false, // continuous audio stream, no silence suppression
                str0m::rtp::ExtensionValues::default(),
                false, // not nackable for audio
                opus_data.to_vec(),
            );
        }

        Ok(())
    }

    /// Send a text message through the DataChannel.
    pub fn send_datachannel_text(&mut self, text: &str) -> Result<(), WebRTCError> {
        let dc_id = match self.dc_id {
            Some(id) => id,
            None => return Err(WebRTCError::DataChannelError("DataChannel not open".to_string())),
        };

        if let Some(mut channel) = self.rtc.channel(dc_id) {
            channel.write(false, text.as_bytes())
                .map_err(|e| WebRTCError::DataChannelError(format!("DC write failed: {}", e)))?;
        }

        Ok(())
    }
}

/// Drive a single RtcSession's event loop over a TCP connection.
///
/// This function runs as a tokio task for each connected peer.
/// It handles:
/// - TCP read → RFC 4571 decode → str0m handle_input
/// - str0m poll_output → RFC 4571 encode → TCP write
/// - RTP broadcast → str0m write_rtp (video)
/// - Audio broadcast → str0m write_rtp (audio)
/// - Text broadcast → DataChannel write
/// - DataChannel events → input_tx

pub async fn drive_session(
    mut session: RtcSession,
    mut tcp_stream: TcpStream,
    peer_addr: SocketAddr,
    local_addr: SocketAddr,
    shared_state: Arc<SharedState>,
    input_tx: mpsc::UnboundedSender<InputEventData>,
    upload_handler: Arc<Mutex<FileUploadHandler>>,
    clipboard: Arc<Mutex<ClipboardReceiver>>,
    runtime_settings: Arc<RuntimeSettings>,
) {
    let session_id = session.id.clone();
    info!("Session {} drive loop started (peer: {})", session_id, peer_addr);

    // Disable Nagle's algorithm for low-latency RTP delivery
    if let Err(e) = tcp_stream.set_nodelay(true) {
        warn!("Session {} failed to set TCP_NODELAY: {}", session_id, e);
    }

    let last_pong = Arc::new(AtomicU64::new(now_millis()));
    let ctx = EventContext {
        input_tx: &input_tx,
        upload_handler: &upload_handler,
        clipboard: &clipboard,
        runtime_settings: &runtime_settings,
        shared_state: &shared_state,
        last_pong: &last_pong,
    };

    let mut decoder = TcpFrameDecoder::new();
    let mut buf = vec![0u8; 65535];
    let mut rtp_rx = shared_state.subscribe_rtp();
    let mut audio_rx = shared_state.subscribe_audio();
    let mut text_rx = shared_state.subscribe_text();

    // Audio RTP state
    let mut audio_timestamp: u32 = 0;
    let samples_per_frame: u32 = 960; // Opus 20ms @ 48kHz

    // Stats counters
    let mut rtp_fwd_count: u64 = 0;
    let mut audio_fwd_count: u64 = 0;

    // Keepalive settings
    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
    let pong_timeout = Duration::from_secs(45);

    // Initial timeout — will be set by drain_outputs
    let mut next_timeout;

    // Drain initial poll_output to get the first timeout
    match drain_outputs(&mut session, &mut tcp_stream, &ctx).await {
        Ok(t) => next_timeout = t,
        Err(e) => {
            error!("Session {} initial drain failed: {}", session_id, e);
            return;
        }
    }

    loop {
        let delay = next_timeout.saturating_duration_since(Instant::now());
        let mut fatal = false;

        tokio::select! {
            // TCP data from browser
            result = tcp_stream.read(&mut buf) => {
                match result {
                    Ok(0) => {
                        info!("Session {} TCP connection closed", session_id);
                        break;
                    }
                    Ok(n) => {
                        decoder.extend(&buf[..n]);
                        loop {
                            match decoder.next_packet() {
                                Ok(Some(pkt)) => {
                                    let recv = net::Receive {
                                        proto: Protocol::Tcp,
                                        source: peer_addr,
                                        destination: local_addr,
                                        contents: match (&*pkt).try_into() {
                                            Ok(c) => c,
                                            Err(e) => {
                                                debug!("Session {} packet parse error: {}", session_id, e);
                                                continue;
                                            }
                                        },
                                    };
                                    if let Err(e) = session.rtc.handle_input(Input::Receive(Instant::now(), recv)) {
                                        warn!("Session {} handle_input error: {}", session_id, e);
                                        fatal = true;
                                        break;
                                    }
                                }
                                Ok(None) => break,
                                Err(e) => {
                                    warn!("Session {} invalid RFC 4571 frame: {:?}", session_id, e);
                                    fatal = true;
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Session {} TCP read error: {}", session_id, e);
                        break;
                    }
                }
            }

            // Timeout
            _ = tokio::time::sleep(delay) => {
                if let Err(e) = session.rtc.handle_input(Input::Timeout(Instant::now())) {
                    warn!("Session {} timeout error: {}", session_id, e);
                    break;
                }
            }

            // Video RTP from GStreamer broadcast
            result = rtp_rx.recv() => {
                match result {
                    Ok(pkt) => {
                        if session.connected {
                            // Collect this packet + all buffered packets
                            let mut rtp_batch = vec![pkt];
                            while let Ok(pkt) = rtp_rx.try_recv() {
                                rtp_batch.push(pkt);
                            }
                            rtp_fwd_count += rtp_batch.len() as u64;
                            if rtp_fwd_count <= 3 {
                                info!("Session {} fwd video RTP batch: {} pkts (total {})",
                                    session_id, rtp_batch.len(), rtp_fwd_count);
                            }
                            // Write each packet individually with its own
                            // handle_timeout + drain cycle.  str0m's NullPacer
                            // emits at most one packet per timeout→poll cycle,
                            // so we must interleave writes and drains.
                            for pkt in rtp_batch {
                                let _ = session.write_video_rtp(&pkt);
                                let _ = session.rtc.handle_input(Input::Timeout(Instant::now()));
                                match drain_outputs_fast(&mut session, &mut tcp_stream, &ctx).await {
                                    Ok(()) => {}
                                    Err(e) => {
                                        warn!("Session {} RTP drain error: {}", session_id, e);
                                        fatal = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Session {} RTP receiver lagged by {}", session_id, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Session {} RTP channel closed", session_id);
                        break;
                    }
                }
            }

            // Audio from broadcast
            result = audio_rx.recv() => {
                match result {
                    Ok(pkt) => {
                        if session.connected {
                            audio_fwd_count += 1;
                            if audio_fwd_count <= 5 || audio_fwd_count % 2000 == 0 {
                                info!("Session {} fwd audio #{}: {} bytes",
                                    session_id, audio_fwd_count, pkt.data.len());
                            }
                            let _ = session.write_audio_rtp(&pkt.data, audio_timestamp);
                            audio_timestamp = audio_timestamp.wrapping_add(samples_per_frame);
                            // Interleaved drain like video — NullPacer needs
                            // handle_timeout + poll per packet.
                            let _ = session.rtc.handle_input(Input::Timeout(Instant::now()));
                            match drain_outputs_fast(&mut session, &mut tcp_stream, &ctx).await {
                                Ok(()) => {}
                                Err(e) => {
                                    warn!("Session {} audio drain error: {}", session_id, e);
                                    fatal = true;
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Session {} audio receiver lagged by {}", session_id, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Text messages (cursor, clipboard, stats) → DataChannel
            result = text_rx.recv() => {
                match result {
                    Ok(msg) => {
                        if session.connected {
                            let _ = session.send_datachannel_text(&msg);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }

            // Keepalive ping
            _ = ping_interval.tick() => {
                if session.connected {
                    let _ = session.send_datachannel_text("ping");
                }
            }
        }

        // After any event, drain str0m outputs
        if fatal {
            break;
        }
        if session.connected {
            let last = last_pong.load(Ordering::Relaxed);
            let now = now_millis();
            if now.saturating_sub(last) > pong_timeout.as_millis() as u64 {
                warn!("Session {} pong timeout; closing connection", session_id);
                break;
            }
        }
        // str0m Sans-I/O requires a Timeout input to timestamp queued RTP
        // packets so the pacer can emit them.  Without this, write_rtp()
        // packets sit in the send queue with a sentinel timestamp and are
        // never transmitted.
        let _ = session.rtc.handle_input(Input::Timeout(Instant::now()));
        match drain_outputs(&mut session, &mut tcp_stream, &ctx).await {
            Ok(t) => next_timeout = t,
            Err(e) => {
                warn!("Session {} drain error: {}", session_id, e);
                break;
            }
        }
    }

    info!("Session {} drive loop ended", session_id);
    shared_state.decrement_webrtc_sessions();
}

/// Drain all pending str0m outputs: transmit packets, handle events, get next timeout.
///
/// str0m's NullPacer emits one RTP packet per handle_timeout→poll_output cycle.
/// We loop: poll_output until Timeout, then re-trigger handle_input(Timeout) to
/// refresh the pacer's queue state, and poll again.  We stop when a cycle
/// produces no Transmit outputs (only Timeout).
async fn drain_outputs(
    session: &mut RtcSession,
    tcp_stream: &mut TcpStream,
    ctx: &EventContext<'_>,
) -> Result<Instant, WebRTCError> {
    let mut next_timeout;
    let mut cycles = 0u32;
    const MAX_DRAIN_CYCLES: u32 = 512;

    loop {
        let mut had_transmit = false;

        // Inner loop: drain poll_output until Timeout
        loop {
            match session.rtc.poll_output() {
                Ok(Output::Transmit(t)) => {
                    had_transmit = true;
                    let framed = frame_packet(&t.contents);
                    if let Err(e) = tcp_stream.write_all(&framed).await {
                        return Err(WebRTCError::ConnectionFailed(format!("TCP write: {}", e)));
                    }
                }
                Ok(Output::Event(event)) => {
                    handle_event(session, event, ctx);
                }
                Ok(Output::Timeout(t)) => {
                    next_timeout = t;
                    break;
                }
                Err(e) => {
                    return Err(WebRTCError::ConnectionFailed(format!("poll_output: {}", e)));
                }
            }
        }

        cycles += 1;
        if !had_transmit || cycles >= MAX_DRAIN_CYCLES {
            break;
        }

        // Re-trigger timeout so the NullPacer refreshes its queue snapshot
        // and can emit the next queued packet.
        if let Err(e) = session.rtc.handle_input(Input::Timeout(Instant::now())) {
            warn!("Session {} drain re-trigger error: {}", session.id, e);
            break;
        }
    }

    Ok(next_timeout)
}

/// Lightweight drain: emit pending Transmit outputs for a single RTP packet.
///
/// Used in the per-packet interleaved write+drain loop for video RTP.
/// Handles events inline so session state stays consistent.
async fn drain_outputs_fast(
    session: &mut RtcSession,
    tcp_stream: &mut TcpStream,
    ctx: &EventContext<'_>,
) -> Result<(), WebRTCError> {
    loop {
        match session.rtc.poll_output() {
            Ok(Output::Transmit(t)) => {
                let framed = frame_packet(&t.contents);
                if let Err(e) = tcp_stream.write_all(&framed).await {
                    return Err(WebRTCError::ConnectionFailed(format!("TCP write: {}", e)));
                }
            }
            Ok(Output::Event(event)) => {
                handle_event(session, event, ctx);
            }
            Ok(Output::Timeout(_)) => break,
            Err(e) => {
                return Err(WebRTCError::ConnectionFailed(format!("poll_output: {}", e)));
            }
        }
    }
    Ok(())
}

/// Context passed to event handlers so they can dispatch DataChannel messages.
struct EventContext<'a> {
    input_tx: &'a mpsc::UnboundedSender<InputEventData>,
    upload_handler: &'a Arc<Mutex<FileUploadHandler>>,
    clipboard: &'a Arc<Mutex<ClipboardReceiver>>,
    runtime_settings: &'a Arc<RuntimeSettings>,
    shared_state: &'a Arc<SharedState>,
    last_pong: &'a Arc<AtomicU64>,
}

/// Handle a str0m event.
fn handle_event(session: &mut RtcSession, event: Event, ctx: &EventContext) {
    match event {
        Event::Connected => {
            session.connected = true;
            info!("Session {} WebRTC connected", session.id);
        }

        Event::MediaAdded(media) => {
            match media.kind {
                MediaKind::Video => {
                    session.video_mid = Some(media.mid);
                    // Discover negotiated H.264 PT from codec config
                    for p in session.rtc.codec_config().params() {
                        if p.spec().codec == str0m::format::Codec::H264 {
                            session.video_pt = Some(p.pt());
                            info!("Session {} video PT: {:?} (H264)", session.id, p.pt());
                            break;
                        }
                    }
                    info!("Session {} video mid: {:?}", session.id, media.mid);
                }
                MediaKind::Audio => {
                    session.audio_mid = Some(media.mid);
                    // Discover negotiated Opus PT from codec config
                    for p in session.rtc.codec_config().params() {
                        if p.spec().codec == str0m::format::Codec::Opus {
                            session.audio_pt = Some(p.pt());
                            info!("Session {} audio PT: {:?}", session.id, p.pt());
                            break;
                        }
                    }
                    info!("Session {} audio mid: {:?}", session.id, media.mid);
                }
            }
        }

        Event::IceConnectionStateChange(state) => {
            info!("Session {} ICE state: {:?}", session.id, state);
            if state == IceConnectionState::Disconnected {
                session.connected = false;
            }
        }

        Event::ChannelOpen(id, label) => {
            session.dc_id = Some(id);
            info!("Session {} DataChannel '{}' opened (id={:?})", session.id, label, id);
            ctx.shared_state.datachannel_open_count.fetch_add(1, Ordering::Relaxed);
        }

        Event::ChannelData(data) => {
            handle_datachannel_data(session, data, ctx);
        }

        Event::ChannelClose(id) => {
            if session.dc_id == Some(id) {
                session.dc_id = None;
            }
            info!("Session {} DataChannel closed (id={:?})", session.id, id);
        }

        Event::KeyframeRequest(_) => {
            ctx.shared_state.request_keyframe();
        }

        _ => {
            debug!("Session {} unhandled event: {:?}", session.id, event);
        }
    }
}

/// Handle incoming DataChannel data — reuses the existing input parsing logic.
fn handle_datachannel_data(session: &mut RtcSession, data: ChannelData, ctx: &EventContext) {
    if data.binary {
        // Binary data → file upload handler
        ctx.upload_handler.lock().unwrap_or_else(|e| e.into_inner())
            .handle_binary(&data.data);
        return;
    }

    let text = match std::str::from_utf8(&data.data) {
        Ok(t) => t,
        Err(e) => {
            debug!("Session {} DC invalid UTF-8: {}", session.id, e);
            return;
        }
    };

    if text.starts_with("pong") {
        ctx.last_pong.store(now_millis(), Ordering::Relaxed);
        return;
    }

    // Try specialized handlers first
    if ctx.upload_handler.lock().unwrap_or_else(|e| e.into_inner()).handle_control_message(text) {
        return;
    }
    if ctx.clipboard.lock().unwrap_or_else(|e| e.into_inner()).handle_message(text) {
        return;
    }
    if ctx.shared_state.handle_command_message(text) {
        return;
    }
    if text.starts_with("SETTINGS,") {
        let payload = text.trim_start_matches("SETTINGS,");
        ctx.runtime_settings.apply_settings_json(payload);
        return;
    }
    if ctx.runtime_settings.handle_simple_message(text) {
        return;
    }
    if text == "kr" {
        let _ = ctx.input_tx.send(InputEventData {
            event_type: InputEvent::KeyboardReset,
            ..Default::default()
        });
        return;
    }
    if text.starts_with("s,") || text.starts_with("SET_NATIVE_CURSOR_RENDERING,") {
        return;
    }
    if text.starts_with("r,") {
        let payload = text.trim_start_matches("r,");
        if let Some((w, h)) = payload.split_once('x') {
            if let (Ok(width), Ok(height)) = (w.parse::<u32>(), h.parse::<u32>()) {
                if width > 0 && height > 0 && width <= 7680 && height <= 4320 {
                    ctx.shared_state.resize_display(width, height);
                }
            }
        }
        return;
    }
    if text.starts_with("_arg_fps,") {
        if let Ok(fps) = text.trim_start_matches("_arg_fps,").parse::<u32>() {
            ctx.runtime_settings.set_target_fps(fps);
        }
        return;
    }
    if text.starts_with("_f,") {
        if let Ok(fps) = text.trim_start_matches("_f,").parse::<u32>() {
            ctx.shared_state.update_client_fps(fps);
        }
        return;
    }
    if text.starts_with("_l,") {
        if let Ok(latency) = text.trim_start_matches("_l,").parse::<u64>() {
            ctx.shared_state.update_client_latency(latency);
        }
        return;
    }
    if text.starts_with("_stats_video,") {
        ctx.shared_state.update_webrtc_stats("video", text.trim_start_matches("_stats_video,"));
        return;
    }
    if text.starts_with("_stats_audio,") {
        ctx.shared_state.update_webrtc_stats("audio", text.trim_start_matches("_stats_audio,"));
        return;
    }
    if text.starts_with("focus,") {
        if let Ok(window_id) = text.trim_start_matches("focus,").parse::<u32>() {
            let mut event = InputEventData::default();
            event.event_type = InputEvent::WindowFocus;
            event.window_id = window_id;
            let _ = ctx.input_tx.send(event);
        }
        return;
    }
    if text.starts_with("close,") {
        if let Ok(window_id) = text.trim_start_matches("close,").parse::<u32>() {
            let mut event = InputEventData::default();
            event.event_type = InputEvent::WindowClose;
            event.window_id = window_id;
            let _ = ctx.input_tx.send(event);
        }
        return;
    }

    // Fall through to input event parsing (mouse, keyboard, etc.)
    match InputDataChannel::parse_input_text(text) {
        Ok(event) => {
            let _ = ctx.input_tx.send(event);
        }
        Err(e) => {
            debug!("Session {} DC parse error: {}", session.id, e);
        }
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
