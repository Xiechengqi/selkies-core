//! selkies-core - Main entry point
//!
//! A high-performance WebRTC streaming solution for X11 desktops using GStreamer.

mod args;
mod config;
mod audio;
mod file_upload;
mod clipboard;
mod runtime_settings;
mod system_clipboard;
mod transport;
mod input;
mod web;
mod display;
mod gstreamer;
mod webrtc;

use args::Args;
use clap::Parser;
use config::Config;
use audio::{run_audio_capture, AudioConfig as RuntimeAudioConfig};
use input::{InputConfig, InputEventData, InputInjector};
use display::{DisplayManager, DisplayManagerConfig};
use base64::Engine;
use image::ImageEncoder;
use log::{info, error, warn, debug};
use xxhash_rust::xxh64::xxh64;
use std::ffi::CString;
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::signal;
use tokio::task;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use x11rb::xcb_ffi::XCBConnection;
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{ConnectionExt as XFixesConnectionExt, CursorNotify, CursorNotifyMask};
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt, CreateGCAux, Rectangle};
use gstreamer::PipelineConfig;
use webrtc::SessionManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize logging with noise filtering for third-party WebRTC crates
    let log_level = if args.verbose { "debug" } else { "info" };
    env_logger::Builder::new()
        .parse_filters(&std::env::var("SELKIES_LOG").unwrap_or_else(|_| log_level.to_string()))
        .filter_module("webrtc_ice", log::LevelFilter::Error)
        .filter_module("webrtc_dtls", log::LevelFilter::Error)
        .filter_module("webrtc_mdns", log::LevelFilter::Error)
        .init();

    info!("selkies-core v{}", env!("CARGO_PKG_VERSION"));
    info!("Starting Selkies streaming core...");

    // Load configuration
    let mut config = match args.load_config() {
        Ok(cfg) => {
            info!("Loaded configuration from {:?}", args.config);
            cfg
        }
        Err(e) => {
            warn!("Failed to load config: {}, using defaults", e);
            Config::default()
        }
    };

    // Apply command line overrides for X11 management
    if args.no_auto_x11 {
        config.display.auto_x11 = false;
    }
    if let Some(ref backend) = args.x11_backend {
        config.display.x11_backend = backend.clone();
    }
    if let Some(ref range_str) = args.x11_display_range {
        if let Some((start, end)) = parse_display_range(range_str) {
            config.display.x11_display_range = [start, end];
        } else {
            warn!("Invalid display range format: {}, using default", range_str);
        }
    }
    if let Some(timeout) = args.x11_startup_timeout {
        config.display.x11_startup_timeout = timeout;
    }

    // Initialize display manager (auto-detect or create X11 display)
    let display_manager = match DisplayManager::new(DisplayManagerConfig {
        preferred_display: config.display.display.clone(),
        auto_x11: config.display.auto_x11,
        x11_backend: config.display.x11_backend.clone(),
        x11_display_range: config.display.x11_display_range,
        x11_startup_timeout: config.display.x11_startup_timeout,
        x11_extra_args: config.display.x11_extra_args.clone(),
        window_manager: config.display.window_manager.clone(),
        width: config.display.width,
        height: config.display.height,
    }) {
        Ok(manager) => {
            info!("Display manager initialized successfully");
            manager
        }
        Err(e) => {
            error!("Failed to initialize display manager: {}", e);
            return Err(e.into());
        }
    };

    // Update config with the actual display being used
    config.display.display = display_manager.display().to_string();
    info!("Using display: {}", config.display.display);

    if let Some(port) = args.http_port {
        info!("Overriding HTTP port to {}", port);
        config.http.port = port;
    }

    apply_basic_auth_overrides(&mut config, &args);
    apply_input_overrides(&mut config, &args);
    apply_webrtc_profile(&mut config, &args);
    apply_webrtc_network_overrides(&mut config, &args);

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Invalid configuration: {}", e);
        return Err(e.into());
    }

    // Initialize GStreamer for WebRTC streaming
    if config.webrtc.enabled {
        info!("Initializing GStreamer for WebRTC streaming...");
        if let Err(e) = gstreamer::init() {
            error!("Failed to initialize GStreamer: {}", e);
            return Err(e.into());
        } else {
            info!("GStreamer initialized successfully");
            // List available encoders
            let encoders = gstreamer::encoder::list_available_encoders();
            info!("Available encoders: {} found", encoders.len());
            for (name, codec, hw_type) in encoders.iter().take(5) {
                debug!("  - {} ({:?}, {:?})", name, codec, hw_type);
            }
        }
    }

    // Create input channel and shared state
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<InputEventData>();
    let ui_config = config::ui::UiConfig::from_env(&config);
    let runtime_settings = Arc::new(runtime_settings::RuntimeSettings::new(&config));
    let state = Arc::new(web::SharedState::new(
        config.clone(),
        ui_config,
        input_tx.clone(),
        runtime_settings.clone(),
    ));
    let running = Arc::new(AtomicBool::new(true));
    let clipboard_running = running.clone();
    let clipboard_state = state.clone();
    let mut clipboard_handle = task::spawn_blocking(move || {
        let mut last_seen_hash: Option<u64> = None;
        while clipboard_running.load(Ordering::Relaxed) {
            let binary_enabled = clipboard_state.runtime_settings.binary_clipboard_enabled();
            if binary_enabled {
                if let Some((mime, data)) = system_clipboard::read_binary() {
                    let mut hash = xxh64(mime.as_bytes(), 0);
                    hash = xxh64(&data, hash);
                    let last_written = clipboard_state.last_clipboard_hash();
                    if last_seen_hash != Some(hash) && last_written != Some(hash) {
                        clipboard_state.set_clipboard_binary(mime, data);
                        last_seen_hash = Some(hash);
                    }
                } else if let Some(text) = system_clipboard::read_text() {
                    let data = text.into_bytes();
                    let mut hash = xxh64(b"text/plain", 0);
                    hash = xxh64(&data, hash);
                    let last_written = clipboard_state.last_clipboard_hash();
                    if last_seen_hash != Some(hash) && last_written != Some(hash) {
                        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                        clipboard_state.set_clipboard(encoded);
                        last_seen_hash = Some(hash);
                    }
                }
            } else if let Some(text) = system_clipboard::read_text() {
                let data = text.into_bytes();
                let mut hash = xxh64(b"text/plain", 0);
                hash = xxh64(&data, hash);
                let last_written = clipboard_state.last_clipboard_hash();
                if last_seen_hash != Some(hash) && last_written != Some(hash) {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
                    clipboard_state.set_clipboard(encoded);
                    last_seen_hash = Some(hash);
                }
            }

            std::thread::sleep(Duration::from_millis(1000));
        }
    });

    // Create WebRTC session manager if enabled
    let session_manager = if config.webrtc.enabled {
        info!("Creating WebRTC session manager...");
        let disable_data_channel = env::var("SELKIES_DISABLE_DATA_CHANNEL")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if disable_data_channel {
            warn!("WebRTC data channels disabled (SELKIES_DISABLE_DATA_CHANNEL)");
        }
        let manager = Arc::new(SessionManager::new(
            config.webrtc.clone(),
            input_tx.clone(),
            file_upload::FileUploadSettings::from_config(&config),
            runtime_settings.clone(),
            state.clone(),
            10, // max 10 concurrent sessions
            !disable_data_channel,
        ));
        info!("WebRTC session manager created");
        Some(manager)
    } else {
        None
    };

    // Forward RTP packets from GStreamer broadcast to WebRTC sessions
    let _rtp_forward_handle = if let Some(manager) = session_manager.clone() {
        let mut rtp_rx = state.subscribe_rtp();
        let mgr = manager.clone();
        Some(task::spawn(async move {
            loop {
                match rtp_rx.recv().await {
                    Ok(packet) => {
                        mgr.broadcast_rtp(&packet).await;
                    }
                    Err(RecvError::Lagged(n)) => {
                        warn!("RTP forward: broadcast channel lagged, dropped {} packets", n);
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }))
    } else {
        None
    };

    // Forward text messages (cursor, clipboard, stats) to WebRTC sessions via data channel
    let _text_forward_handle = if let Some(manager) = session_manager.clone() {
        let mut text_rx = state.subscribe_text();
        let mgr = manager.clone();
        Some(task::spawn(async move {
            loop {
                match text_rx.recv().await {
                    Ok(message) => {
                        mgr.broadcast_text(&message).await;
                    }
                    Err(RecvError::Lagged(n)) => {
                        warn!("Text forward: broadcast channel lagged, dropped {} messages", n);
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }))
    } else {
        None
    };

    // Start GStreamer pipeline for WebRTC (if enabled)
    let gst_handle = if config.webrtc.enabled && session_manager.is_some() {
        let gst_state = state.clone();
        let gst_running = running.clone();
        let mut pipeline_config = PipelineConfig::from(&config.webrtc);
        // Use the display created/detected by display manager
        pipeline_config.display = config.display.display.clone();

        Some(task::spawn_blocking(move || {
            info!("Starting GStreamer pipeline...");

            let mut pipeline = match gstreamer::VideoPipeline::new(pipeline_config.clone()) {
                Ok(p) => {
                    info!("GStreamer pipeline created successfully");
                    p
                }
                Err(e) => {
                    error!("Failed to create GStreamer pipeline: {}", e);
                    return Err(format!("Pipeline creation failed: {}", e));
                }
            };

            if let Err(e) = pipeline.start() {
                error!("Failed to start GStreamer pipeline: {}", e);
                return Err(format!("Pipeline start failed: {}", e));
            }

            let mut current_bitrate = gst_state.runtime_settings.video_bitrate_kbps();
            let mut current_keyframe = gst_state.runtime_settings.keyframe_interval();
            pipeline.set_bitrate(current_bitrate);
            pipeline.set_keyframe_interval(current_keyframe);
            info!("GStreamer pipeline started, streaming RTP packets...");
            let mut packet_count = 0u64;
            // Keyframe cache: collect RTP packets belonging to the current IDR frame
            let mut keyframe_packets: Vec<Vec<u8>> = Vec::new();
            let mut collecting_keyframe = false;
            // Track the RTP timestamp of the keyframe being collected
            let mut keyframe_ts: u32 = 0;
            let mut pipeline_paused = false;

            while gst_running.load(Ordering::Relaxed) {
                // Pause pipeline when no clients are connected to save CPU
                let session_count = gst_state.webrtc_session_count.load(Ordering::Relaxed);
                if session_count == 0 && !pipeline_paused {
                    info!("No WebRTC clients connected, pausing pipeline");
                    let _ = pipeline.pause();
                    pipeline_paused = true;
                } else if session_count > 0 && pipeline_paused {
                    info!("WebRTC client connected, resuming pipeline");
                    let _ = pipeline.resume();
                    pipeline_paused = false;
                    pipeline.request_keyframe();
                }

                if pipeline_paused {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }

                // Check for keyframe request
                if gst_state.take_keyframe_request() || gst_state.runtime_settings.take_keyframe_request() {
                    pipeline.request_keyframe();
                    debug!("Keyframe requested");
                }

                let desired_bitrate = gst_state.runtime_settings.video_bitrate_kbps();
                if desired_bitrate != current_bitrate {
                    current_bitrate = desired_bitrate;
                    pipeline.set_bitrate(current_bitrate);
                }
                let desired_keyframe = gst_state.runtime_settings.keyframe_interval();
                if desired_keyframe != current_keyframe {
                    current_keyframe = desired_keyframe;
                    pipeline.set_keyframe_interval(current_keyframe);
                }

                // Check for pending display resize
                if let Some((w, h)) = gst_state.take_pending_resize() {
                    info!("Display resize to {}x{} requested, stopping pipeline...", w, h);
                    let _ = pipeline.stop();
                    std::thread::sleep(Duration::from_millis(200));
                    gst_state.apply_xrandr_resize(w, h);
                    std::thread::sleep(Duration::from_millis(100));
                    match gstreamer::VideoPipeline::new(pipeline_config.clone()) {
                        Ok(new_p) => {
                            pipeline = new_p;
                            if let Err(e) = pipeline.start() {
                                error!("Failed to restart pipeline: {}", e);
                                break;
                            }
                            pipeline.set_bitrate(current_bitrate);
                            pipeline.set_keyframe_interval(current_keyframe);
                            keyframe_packets.clear();
                            collecting_keyframe = false;
                            info!("Pipeline rebuilt after resize to {}x{}", w, h);
                        }
                        Err(e) => {
                            error!("Failed to rebuild pipeline: {}", e);
                            break;
                        }
                    }
                }

                // Pull RTP packet from pipeline (block up to 100ms)
                if let Some(sample) = pipeline.try_pull_sample_timeout(100) {
                    if let Some(buffer) = sample.buffer() {
                        if let Ok(map) = buffer.map_readable() {
                            let packet = map.as_slice().to_vec();

                            // Detect H264 keyframe packets and cache them
                            if packet.len() >= 13 {
                                let rtp_ts = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
                                let hdr_len = {
                                    let cc = (packet[0] & 0x0F) as usize;
                                    let mut l = 12 + cc * 4;
                                    if (packet[0] & 0x10) != 0 && packet.len() >= l + 4 {
                                        let ext = u16::from_be_bytes([packet[l+2], packet[l+3]]) as usize;
                                        l += 4 + ext * 4;
                                    }
                                    l
                                };
                                if packet.len() > hdr_len {
                                    let nal_byte = packet[hdr_len];
                                    let nal_type = nal_byte & 0x1F;

                                    // SPS(7), PPS(8), IDR(5), or STAP-A(24) start a keyframe
                                    let is_kf_start = nal_type == 7 || nal_type == 8
                                        || nal_type == 5 || nal_type == 24
                                        || (nal_type == 28 && packet.len() > hdr_len + 1 && {
                                            let fu_nal = packet[hdr_len + 1] & 0x1F;
                                            let fu_s = (packet[hdr_len + 1] & 0x80) != 0;
                                            fu_s && fu_nal == 5
                                        });
                                    if is_kf_start && !collecting_keyframe {
                                        collecting_keyframe = true;
                                        keyframe_ts = rtp_ts;
                                        keyframe_packets.clear();
                                    }
                                    if collecting_keyframe {
                                        if rtp_ts == keyframe_ts {
                                            keyframe_packets.push(packet.clone());
                                        } else {
                                            // New timestamp = new frame, stop collecting
                                            collecting_keyframe = false;
                                            let cached = keyframe_packets.clone();
                                            debug!("Cached keyframe: {} RTP packets, total {} bytes",
                                                cached.len(), cached.iter().map(|p| p.len()).sum::<usize>());
                                            gst_state.set_keyframe_cache(cached);
                                        }
                                    }
                                }
                            }

                            // Broadcast to all WebRTC sessions
                            gst_state.broadcast_rtp(packet);

                            packet_count += 1;
                            if packet_count % 1000 == 0 {
                                debug!("Sent {} RTP packets", packet_count);
                            }
                        }
                    }
                } else {
                    // try_pull_sample_timeout already blocks up to 100ms, no extra sleep needed
                }
            }

            info!("GStreamer pipeline stopped");
            let _ = pipeline.stop();
            Ok::<(), String>(())
        }))
    } else {
        None
    };

    // Start input injection loop
    let input_display = config.display.display.clone();
    let input_config = InputConfig {
        enable_keyboard: config.input.enable_keyboard,
        enable_mouse: config.input.enable_mouse,
        mouse_sensitivity: config.input.mouse_sensitivity,
    };
    let input_running = running.clone();
    let mut input_handle = task::spawn_blocking(move || {
        while input_running.load(Ordering::Relaxed) {
            let display_cstr = match CString::new(input_display.as_str()) {
                Ok(value) => value,
                Err(e) => {
                    error!("Invalid display string: {}", e);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let (conn, screen_num) = match XCBConnection::connect(Some(display_cstr.as_c_str())) {
                Ok(result) => result,
                Err(e) => {
                    error!("X11 connection failed: {}", e);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let conn = Arc::new(conn);
            let screen_num = screen_num as i32;

            let mut injector = match InputInjector::new(conn, screen_num, input_config.clone()) {
                Ok(value) => value,
                Err(e) => {
                    error!("Input injector init failed: {}", e);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };

            while input_running.load(Ordering::Relaxed) {
                match input_rx.blocking_recv() {
                    Some(event) => {
                        if let Err(e) = injector.process_event(event) {
                            error!("Input event failed: {}", e);
                        }
                    }
                    None => break,
                }
            }
        }

        Ok::<(), String>(())
    });

    // Start audio streaming loop
    let audio_running = running.clone();
    let audio_sender = state.audio_sender.clone();
    let base_audio_config = RuntimeAudioConfig {
        sample_rate: config.audio.sample_rate,
        channels: config.audio.channels,
        bitrate: config.audio.bitrate,
    };
    let audio_state = state.clone();
    let mut audio_handle = if config.audio.enabled {
        Some(task::spawn_blocking(move || {
            let mut current_bitrate = base_audio_config.bitrate;
            let mut config = base_audio_config.clone();
            let audio_active = Arc::new(AtomicBool::new(true));
            loop {
                if !audio_running.load(Ordering::Relaxed) {
                    break;
                }
                if let Some(new_bitrate) = audio_state.runtime_settings.take_audio_bitrate_update() {
                    current_bitrate = new_bitrate;
                    config = config.with_bitrate(new_bitrate);
                    info!("Restarting audio capture with bitrate {}", new_bitrate);
                }

                audio_active.store(true, Ordering::Relaxed);
                let monitor_running = audio_running.clone();
                let monitor_active = audio_active.clone();
                let monitor_settings = audio_state.runtime_settings.clone();
                let monitor = std::thread::spawn(move || {
                    while monitor_running.load(Ordering::Relaxed) {
                        if monitor_settings.audio_bitrate_dirty() {
                            monitor_active.store(false, Ordering::Relaxed);
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                });

                if let Err(e) = run_audio_capture(config.clone(), audio_sender.clone(), audio_active.clone()) {
                    error!("Audio capture error: {}", e);
                }
                let _ = monitor.join();
                if audio_state.runtime_settings.take_audio_bitrate_update().is_some() {
                    continue;
                }
                break;
            }
        }))
    } else {
        None
    };

    // Start cursor tracking loop
    let cursor_hidden = Arc::new(AtomicBool::new(false));
    let cursor_running = running.clone();
    let cursor_state = state.clone();
    let cursor_display = config.display.display.clone();
    let cursor_hidden_cursor = Arc::clone(&cursor_hidden);
    let cursor_handle: Option<tokio::task::JoinHandle<Result<(), String>>> = Some(task::spawn_blocking(move || {
        info!("Cursor tracking loop started for display {}", cursor_display);
        while cursor_running.load(Ordering::Relaxed) {
            let display_cstr = match CString::new(cursor_display.as_str()) {
                Ok(value) => value,
                Err(e) => {
                    error!("Invalid display string: {}", e);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let (conn, screen_num) = match XCBConnection::connect(Some(display_cstr.as_c_str())) {
                Ok(result) => {
                    info!("Cursor tracking: X11 connection established");
                    result
                },
                Err(e) => {
                    error!("X11 connection failed for cursor tracking: {}", e);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let conn = Arc::new(conn);
            let screen = &conn.setup().roots[screen_num as usize];
            let root = screen.root;
            if hide_cursor(&conn, root) {
                cursor_hidden_cursor.store(true, Ordering::Relaxed);
                info!("Cursor tracking: Hardware cursor hidden");
            } else {
                warn!("Cursor tracking: Failed to hide hardware cursor");
            }
            if let Err(e) = conn.xfixes_select_cursor_input(root, CursorNotifyMask::DISPLAY_CURSOR) {
                warn!("Cursor tracking: Failed to select cursor input events: {:?}", e);
            }
            let mut last = (i16::MIN, i16::MIN);
            let mut last_serial = u32::MAX; // Initialize to MAX so first cursor is always sent
            let mut cursor_msg_count = 0u64;
            let mut first_message_sent = false;
            let mut cursor_override: Option<String> = None;
            let mut window_check_counter: u32 = 0;
            let mut last_has_windows: Option<bool> = None;
            info!("Cursor tracking: Starting cursor position polling loop");
            while cursor_running.load(Ordering::Relaxed) {
                while let Ok(Some(event)) = conn.poll_for_event() {
                    if let x11rb::protocol::Event::XfixesCursorNotify(ev) = event {
                        if ev.subtype == CursorNotify::DISPLAY_CURSOR {
                            if let Ok(name_cookie) = conn.get_atom_name(ev.name) {
                                if let Ok(name_reply) = name_cookie.reply() {
                                    if let Ok(name_str) = std::str::from_utf8(&name_reply.name) {
                                        cursor_override = Some(map_cursor_name_to_css(name_str));
                                    }
                                }
                            }
                        }
                    }
                }
                if let Ok(cookie) = conn.query_pointer(root) {
                    if let Ok(reply) = cookie.reply() {
                        let mut image_ok = false;
                        let pos = (reply.root_x, reply.root_y);
                        let mut payload = format!(
                            r#"cursor,{{"x":{},"y":{},"visible":true,"handle":0}}"#,
                            reply.root_x, reply.root_y
                        );
                        let mut image_changed = false;
                        match conn.xfixes_get_cursor_image() {
                            Ok(img_cookie) => {
                                match img_cookie.reply() {
                                    Ok(img) => {
                                        let is_transparent = img.cursor_image.iter().all(|pixel| *pixel == 0);
                                        if !first_message_sent {
                                            info!("Cursor tracking: First cursor check - serial={}, last_serial={}", img.cursor_serial, last_serial);
                                        }
                                        if img.cursor_serial != last_serial {
                                            last_serial = img.cursor_serial;
                                            image_changed = true;
                                            if let Some(encoded) = encode_cursor_png(&img) {
                                                image_ok = true;
                                                if !first_message_sent {
                                                    info!("Cursor tracking: Encoded cursor image (serial={}, size={}x{}, hotspot=({},{}), data_len={})",
                                                        img.cursor_serial, img.width, img.height, img.xhot, img.yhot, encoded.len());
                                                }
                                                let override_field = if is_transparent { r#","override":"none""# } else { "" };
                                                payload = format!(
                                                    r#"cursor,{{"x":{},"y":{},"visible":true,"handle":{},"curdata":"{}","hotx":{},"hoty":{}{}}}"#,
                                                    reply.root_x,
                                                    reply.root_y,
                                                    img.cursor_serial,
                                                    encoded,
                                                    img.xhot,
                                                    img.yhot,
                                                    override_field
                                                );
                                            } else if !first_message_sent {
                                                info!("Cursor tracking: Failed to encode cursor image");
                                            }
                                        } else {
                                            image_ok = true;
                                            if is_transparent {
                                                payload = format!(
                                                    r#"cursor,{{"x":{},"y":{},"visible":true,"handle":{},"override":"none"}}"#,
                                                    reply.root_x,
                                                    reply.root_y,
                                                    img.cursor_serial
                                                );
                                            } else {
                                                payload = format!(
                                                    r#"cursor,{{"x":{},"y":{},"visible":true,"handle":{}}}"#,
                                                    reply.root_x,
                                                    reply.root_y,
                                                    img.cursor_serial
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        if !first_message_sent || cursor_msg_count % 500 == 0 {
                                            warn!("Cursor tracking: Failed to get cursor image reply: {:?}", e);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                if !first_message_sent {
                                    info!("Cursor tracking: Failed to call xfixes_get_cursor_image: {:?}", e);
                                }
                            }
                        }
                        if !image_ok {
                            let override_cursor = cursor_override.as_deref().unwrap_or("default");
                            payload = format!(
                                r#"cursor,{{"x":{},"y":{},"visible":true,"handle":0,"override":"{}"}}"#,
                                reply.root_x,
                                reply.root_y,
                                override_cursor
                            );
                            image_ok = true;
                        }

                        if image_ok && !cursor_hidden_cursor.load(Ordering::Relaxed) {
                            debug!("Cursor image available but cursor not hidden; captured cursor may double-render.");
                        }
                        if pos != last || image_changed || !first_message_sent {
                            last = pos;
                            cursor_msg_count += 1;
                            if cursor_msg_count == 1 {
                                info!("Cursor tracking: Sending first cursor message at ({}, {})", pos.0, pos.1);
                            }
                            // Ignore send errors - they happen when no clients are connected
                            cursor_state.update_cursor_message(payload.clone());
                            let _ = cursor_state.text_sender.send(payload);
                            first_message_sent = true;
                            if cursor_msg_count % 100 == 0 {
                                debug!("Cursor tracking: Sent {} cursor messages", cursor_msg_count);
                            }
                        }
                    }
                }

                // Window detection (check every 2 seconds: 100ms * 20 = 2000ms)
                window_check_counter = window_check_counter.wrapping_add(1);
                if window_check_counter % 20 == 0 {
                    if let Ok(tree_cookie) = conn.query_tree(root) {
                        if let Ok(tree) = tree_cookie.reply() {
                            let has_windows = !tree.children.is_empty();
                            // Only send message when state changes or on first check
                            if last_has_windows != Some(has_windows) {
                                last_has_windows = Some(has_windows);
                                let payload = format!(r#"window_state,{{"has_windows":{}}}"#, has_windows);
                                let _ = cursor_state.text_sender.send(payload);
                                info!("Window state changed: has_windows={}", has_windows);
                            }
                        }
                    }
                }

                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Ok::<(), String>(())
    }));

    // Start WebRTC session cleanup task (if enabled)
    let cleanup_handle = if let Some(ref manager) = session_manager {
        let cleanup_running = running.clone();
        let cleanup_manager = manager.clone();

        Some(task::spawn(async move {
            info!("Starting WebRTC session cleanup task...");
            while cleanup_running.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_secs(30)).await;

                // Clean up stale sessions (disconnected for > 60 seconds)
                cleanup_manager.cleanup_stale_sessions(60).await;
            }
            info!("WebRTC session cleanup task stopped");
        }))
    } else {
        None
    };

    // Start stats sampling loop (CPU/memory)
    let stats_running = running.clone();
    let stats_state = state.clone();
    let stats_broadcast = state.text_sender.clone();
    let mut stats_handle = task::spawn_blocking(move || {
        let mut last_proc = read_proc_jiffies();
        let mut last_total = read_total_jiffies();
        while stats_running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(1));
            let proc = read_proc_jiffies();
            let total = read_total_jiffies();
            if let (Some(prev_proc), Some(prev_total), Some(cur_proc), Some(cur_total)) =
                (last_proc, last_total, proc, total)
            {
                let delta_proc = cur_proc.saturating_sub(prev_proc) as f64;
                let delta_total = cur_total.saturating_sub(prev_total) as f64;
                let cpu = if delta_total > 0.0 {
                    (delta_proc / delta_total) * 100.0
                } else {
                    0.0
                };
                let mem = read_rss_bytes().unwrap_or(0);
                stats_state.update_resource_usage(cpu, mem);
                let stats_payload = stats_state.stats_json();
                let _ = stats_broadcast.send(format!("stats,{}", stats_payload));
                let bandwidth = stats_state.stats.lock().unwrap().bandwidth;
                let kbps = bandwidth.saturating_mul(8) / 1000;
                let system_payload = format!(
                    r#"{{"action":"bitrate","data":"{}"}}"#,
                    kbps
                );
                let _ = stats_broadcast.send(format!("system,{}", system_payload));
            }
            last_proc = proc;
            last_total = total;
        }
        Ok::<(), String>(())
    });

    // Start HTTP server with WebRTC signaling support
    let http_server = web::run_http_server_with_webrtc(
        config.http.port,
        state.clone(),
        session_manager.clone(),
    );

    let mut http_handle = task::spawn(async move {
        if let Err(e) = http_server.await {
            error!("HTTP server error: {}", e);
        }
    });

    // Wait for shutdown signal
    let shutdown = async {
        let _ = signal::ctrl_c().await;
        info!("Shutdown signal received");
    };

    if let Some(handle) = audio_handle.as_mut() {
        tokio::select! {
            _ = shutdown => {
                info!("Initiating graceful shutdown...");
            }
            result = &mut input_handle => {
                log_blocking_task_result("Input loop", result);
            }
            result = &mut stats_handle => {
                log_blocking_task_result("Stats loop", result);
            }
            result = &mut clipboard_handle => {
                log_async_task_result("Clipboard loop", result);
            }
            result = &mut http_handle => {
                log_async_task_result("HTTP server", result);
            }
            result = handle => {
                log_async_task_result("Audio loop", result);
            }
        }
    } else {
        tokio::select! {
            _ = shutdown => {
                info!("Initiating graceful shutdown...");
            }
            result = &mut input_handle => {
                log_blocking_task_result("Input loop", result);
            }
            result = &mut stats_handle => {
                log_blocking_task_result("Stats loop", result);
            }
            result = &mut clipboard_handle => {
                log_async_task_result("Clipboard loop", result);
            }
            result = &mut http_handle => {
                log_async_task_result("HTTP server", result);
            }
        }
    }

    // Cleanup
    running.store(false, Ordering::Relaxed);

    info!("Stopping all tasks...");

    // Stop HTTP server
    if !http_handle.is_finished() {
        http_handle.abort();
        let _ = http_handle.await;
    }

    // Stop GStreamer pipeline
    if let Some(handle) = gst_handle {
        if !handle.is_finished() {
            info!("Stopping GStreamer pipeline...");
            handle.abort();
            let _ = handle.await;
        }
    }

    // Stop WebRTC cleanup task
    if let Some(handle) = cleanup_handle {
        if !handle.is_finished() {
            handle.abort();
            let _ = handle.await;
        }
    }

    // Stop cursor tracking
    if let Some(handle) = cursor_handle {
        if !handle.is_finished() {
            handle.abort();
            let _ = handle.await;
        }
    }

    // Stop stats loop
    if !stats_handle.is_finished() {
        stats_handle.abort();
        let _ = stats_handle.await;
    }

    // Stop clipboard loop
    if !clipboard_handle.is_finished() {
        clipboard_handle.abort();
        let _ = clipboard_handle.await;
    }

    // Stop input loop
    if !input_handle.is_finished() {
        input_handle.abort();
        let _ = input_handle.await;
    }

    // Stop audio loop
    if let Some(handle) = audio_handle {
        if !handle.is_finished() {
            handle.abort();
            let _ = handle.await;
        }
    }

    state.shutdown().await;
    info!("selkies-core stopped");

    Ok(())
}

fn log_blocking_task_result(task: &str, result: Result<Result<(), String>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => warn!("{} stopped unexpectedly", task),
        Ok(Err(err)) => error!("{} failed: {}", task, err),
        Err(err) => error!("{} join error: {}", task, err),
    }
}

fn log_async_task_result(task: &str, result: Result<(), tokio::task::JoinError>) {
    match result {
        Ok(()) => warn!("{} stopped unexpectedly", task),
        Err(err) => error!("{} join error: {}", task, err),
    }
}

fn read_proc_jiffies() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let mut parts = stat.split_whitespace();
    let _pid = parts.next()?;
    let _comm = parts.next()?;
    let _state = parts.next()?;
    let _ppid = parts.next()?;
    let _pgrp = parts.next()?;
    let _session = parts.next()?;
    let _tty_nr = parts.next()?;
    let _tpgid = parts.next()?;
    let _flags = parts.next()?;
    let _minflt = parts.next()?;
    let _cminflt = parts.next()?;
    let _majflt = parts.next()?;
    let _cmajflt = parts.next()?;
    let utime: u64 = parts.next()?.parse().ok()?;
    let stime: u64 = parts.next()?.parse().ok()?;
    Some(utime + stime)
}

fn read_total_jiffies() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let mut lines = stat.lines();
    let cpu_line = lines.next()?;
    let mut total = 0u64;
    for part in cpu_line.split_whitespace().skip(1) {
        if let Ok(val) = part.parse::<u64>() {
            total += val;
        }
    }
    Some(total)
}

fn read_rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let mut parts = statm.split_whitespace();
    let _size = parts.next()?;
    let resident: u64 = parts.next()?.parse().ok()?;
    Some(resident * 4096)
}

fn map_cursor_name_to_css(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("hand") || lower.contains("pointer") {
        "pointer".to_string()
    } else if lower.contains("xterm") || lower.contains("text") || lower.contains("ibeam") {
        "text".to_string()
    } else if lower.contains("cross") {
        "crosshair".to_string()
    } else {
        "default".to_string()
    }
}

fn hide_cursor(conn: &XCBConnection, root: x11rb::protocol::xproto::Window) -> bool {
    match conn.xfixes_hide_cursor(root) {
        Ok(cookie) => {
            if cookie.check().is_ok() {
                return true;
            }
        }
        Err(e) => {
            debug!("Failed to hide cursor via XFixes: {:?}", e);
        }
    }

    let pixmap = match conn.generate_id() {
        Ok(value) => value,
        Err(e) => {
            debug!("Failed to allocate pixmap id for cursor: {:?}", e);
            return false;
        }
    };
    let gc = match conn.generate_id() {
        Ok(value) => value,
        Err(e) => {
            debug!("Failed to allocate gc id for cursor: {:?}", e);
            return false;
        }
    };
    let cursor = match conn.generate_id() {
        Ok(value) => value,
        Err(e) => {
            debug!("Failed to allocate cursor id: {:?}", e);
            return false;
        }
    };

    if conn.create_pixmap(1, pixmap, root, 1, 1).is_err() {
        debug!("Failed to create cursor pixmap");
        return false;
    }

    let gc_aux = CreateGCAux::new().foreground(0).background(0);
    if conn.create_gc(gc, pixmap, &gc_aux).is_err() {
        debug!("Failed to create cursor GC");
        return false;
    }

    let rect = Rectangle {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let _ = conn.poly_fill_rectangle(pixmap, gc, &[rect]);
    let _ = conn.create_cursor(
        cursor,
        pixmap,
        pixmap,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    let _ = conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().cursor(cursor));
    let _ = conn.flush();
    true
}


fn encode_cursor_png(reply: &x11rb::protocol::xfixes::GetCursorImageReply) -> Option<String> {
    use image::{ImageBuffer, Rgba};

    let width = reply.width as u32;
    let height = reply.height as u32;
    if width == 0 || height == 0 {
        return None;
    }

    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    for pixel in reply.cursor_image.iter() {
        let p = *pixel;
        let a = (p >> 24) as u8;
        let r = (p >> 16) as u8;
        let g = (p >> 8) as u8;
        let b = p as u8;
        buf.push(r);
        buf.push(g);
        buf.push(b);
        buf.push(a);
    }

    let image = ImageBuffer::<Rgba<u8>, _>::from_vec(width, height, buf)?;
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    if encoder
        .write_image(
            image.as_raw(),
            width,
            height,
            image::ColorType::Rgba8,
        )
        .is_ok()
    {
        Some(base64::engine::general_purpose::STANDARD.encode(png))
    } else {
        None
    }
}

fn apply_basic_auth_overrides(config: &mut Config, args: &Args) {
    let env_enabled = env_bool("SELKIES_BASIC_AUTH_ENABLED");
    let env_user = env_var("SELKIES_BASIC_AUTH_USER");
    let env_password = env_var("SELKIES_BASIC_AUTH_PASSWORD");

    if let Some(enabled) = args.basic_auth_enabled.or(env_enabled) {
        config.http.basic_auth_enabled = enabled;
    }
    if let Some(user) = args.basic_auth_user.clone().or(env_user) {
        config.http.basic_auth_user = user;
    }
    if let Some(password) = args.basic_auth_password.clone().or(env_password) {
        config.http.basic_auth_password = password;
    }

    if config.http.basic_auth_user.is_empty() {
        config.http.basic_auth_user = env::var("USER").unwrap_or_else(|_| "user".to_string());
    }
}

fn apply_input_overrides(config: &mut Config, args: &Args) {
    let env_binary_clipboard = env_bool("SELKIES_BINARY_CLIPBOARD_ENABLED");
    let env_commands = env_bool("SELKIES_COMMANDS_ENABLED");
    let env_transfers = env_var("SELKIES_FILE_TRANSFERS");
    let env_upload_dir = env_var("SELKIES_UPLOAD_DIR");

    if let Some(enabled) = args.binary_clipboard_enabled.or(env_binary_clipboard) {
        config.input.enable_binary_clipboard = enabled;
    }
    if let Some(enabled) = args.commands_enabled.or(env_commands) {
        config.input.enable_commands = enabled;
    }
    if let Some(list_str) = args.file_transfers.clone().or(env_transfers) {
        config.input.file_transfers = parse_csv_list(&list_str);
    }
    if let Some(upload_dir) = args.upload_dir.clone().or(env_upload_dir) {
        config.input.upload_dir = upload_dir;
    }
}

fn env_var(key: &str) -> Option<String> {
    match env::var(key) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

fn env_bool(key: &str) -> Option<bool> {
    let raw = env::var(key).ok()?;
    match raw.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" => Some(true),
        "false" | "0" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn apply_webrtc_network_overrides(config: &mut Config, args: &Args) {
    let env_trickle = env_bool("SELKIES_WEBRTC_ICE_TRICKLE");
    let env_nat1to1 = env_var("SELKIES_WEBRTC_NAT1TO1");
    let env_ip_url = env_var("SELKIES_WEBRTC_IP_RETRIEVAL_URL");
    let env_epr = env_var("SELKIES_WEBRTC_EPR");
    let env_udp_mux = env_var("SELKIES_WEBRTC_UDP_MUX_PORT");
    let env_tcp_mux = env_var("SELKIES_WEBRTC_TCP_MUX_PORT");
    let env_stun_host = env_var("SELKIES_WEBRTC_STUN_HOST");
    let env_stun_port = env_var("SELKIES_WEBRTC_STUN_PORT");
    let env_turn_host = env_var("SELKIES_WEBRTC_TURN_HOST");
    let env_turn_port = env_var("SELKIES_WEBRTC_TURN_PORT");
    let env_turn_protocol = env_var("SELKIES_WEBRTC_TURN_PROTOCOL");
    let env_turn_tls = env_bool("SELKIES_WEBRTC_TURN_TLS");
    let env_turn_shared_secret = env_var("SELKIES_WEBRTC_TURN_SHARED_SECRET");
    let env_turn_username = env_var("SELKIES_WEBRTC_TURN_USERNAME");
    let env_turn_password = env_var("SELKIES_WEBRTC_TURN_PASSWORD");

    if let Some(trickle) = args.webrtc_ice_trickle.or(env_trickle) {
        config.webrtc.ice_trickle = trickle;
    }

    if let Some(list_str) = args.webrtc_nat1to1.clone().or(env_nat1to1) {
        config.webrtc.nat1to1_ips = parse_csv_list(&list_str);
    }

    if let Some(url) = args.webrtc_ip_retrieval_url.clone().or(env_ip_url) {
        config.webrtc.ip_retrieval_url = url;
    }

    if let Some(range_str) = args.webrtc_ephemeral_udp_port_range.clone().or(env_epr) {
        let parsed = parse_port_range(&range_str);
        if parsed.is_none() {
            warn!("Invalid WebRTC ephemeral UDP port range: {}", range_str);
        }
        config.webrtc.ephemeral_udp_port_range = parsed;
    }

    if let Some(port) = args.webrtc_udp_mux_port.or(parse_u16(&env_udp_mux)) {
        config.webrtc.udp_mux_port = port;
    }

    if let Some(port) = args.webrtc_tcp_mux_port.or(parse_u16(&env_tcp_mux)) {
        config.webrtc.tcp_mux_port = port;
    }

    if let Some(host) = args.webrtc_stun_host.clone().or(env_stun_host) {
        config.webrtc.stun_host = host;
    }
    if let Some(port) = args.webrtc_stun_port.or(parse_u16(&env_stun_port)) {
        config.webrtc.stun_port = port;
    }
    if let Some(host) = args.webrtc_turn_host.clone().or(env_turn_host) {
        config.webrtc.turn_host = host;
    }
    if let Some(port) = args.webrtc_turn_port.or(parse_u16(&env_turn_port)) {
        config.webrtc.turn_port = port;
    }
    if let Some(protocol) = args.webrtc_turn_protocol.clone().or(env_turn_protocol) {
        config.webrtc.turn_protocol = protocol;
    }
    if let Some(tls) = args.webrtc_turn_tls.or(env_turn_tls) {
        config.webrtc.turn_tls = tls;
    }
    if let Some(secret) = args.webrtc_turn_shared_secret.clone().or(env_turn_shared_secret) {
        config.webrtc.turn_shared_secret = secret;
    }
    if let Some(user) = args.webrtc_turn_username.clone().or(env_turn_username) {
        config.webrtc.turn_username = user;
    }
    if let Some(password) = args.webrtc_turn_password.clone().or(env_turn_password) {
        config.webrtc.turn_password = password;
    }
}

fn apply_webrtc_profile(config: &mut Config, args: &Args) {
    let env_profile = env_var("SELKIES_WEBRTC_PROFILE");
    let profile = args
        .webrtc_profile
        .clone()
        .or(env_profile)
        .or_else(|| config.webrtc.network_profile.clone());

    let profile = match profile {
        Some(p) => p.to_ascii_lowercase(),
        None => return,
    };

    match profile.as_str() {
        "lan" => {
            config.webrtc.ice_trickle = true;
            config.webrtc.nat1to1_ips.clear();
            config.webrtc.ip_retrieval_url.clear();
            config.webrtc.ephemeral_udp_port_range = None;
            config.webrtc.udp_mux_port = 0;
            config.webrtc.tcp_mux_port = 0;
        }
        "wan" => {
            config.webrtc.ice_trickle = true;
            if config.webrtc.ice_servers.is_empty() {
                config.webrtc.ice_servers = vec![crate::config::IceServerConfig::default()];
            }
            if config.webrtc.ephemeral_udp_port_range.is_none() {
                config.webrtc.ephemeral_udp_port_range = Some([59000, 59100]);
            }
            if config.webrtc.ip_retrieval_url.is_empty() {
                config.webrtc.ip_retrieval_url = "https://checkip.amazonaws.com".to_string();
            }
            if config.webrtc.stun_host.is_empty() {
                config.webrtc.stun_host = "stun.l.google.com".to_string();
            }
            if config.webrtc.stun_port == 0 {
                config.webrtc.stun_port = 19302;
            }
        }
        _ => {
            warn!("Unknown WebRTC profile: {} (expected \"lan\" or \"wan\")", profile);
        }
    }

    config.webrtc.network_profile = Some(profile);
}

fn parse_csv_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return Vec::new();
    }

    trimmed
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect()
}

fn parse_u16(value: &Option<String>) -> Option<u16> {
    value.as_ref()?.trim().parse::<u16>().ok()
}

fn parse_port_range(value: &str) -> Option<[u16; 2]> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let start = parts[0].trim().parse::<u16>().ok()?;
    let end = parts[1].trim().parse::<u16>().ok()?;
    if start == 0 || end == 0 || start > end {
        return None;
    }
    Some([start, end])
}

/// Parse display range string (e.g., "99-199") into [start, end]
fn parse_display_range(range_str: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = range_str.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    let start = parts[0].trim().parse::<u32>().ok()?;
    let end = parts[1].trim().parse::<u32>().ok()?;

    if start <= end {
        Some((start, end))
    } else {
        None
    }
}
