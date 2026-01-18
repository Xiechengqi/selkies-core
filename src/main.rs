//! selkies-core - Main entry point
//!
//! A high-performance streaming solution for X11 desktops with support for:
//! - WebRTC + GStreamer (default, low-latency)
//! - WebSocket + TurboJPEG (legacy, fallback)

mod args;
mod config;
mod capture;
mod encode;
mod audio;
mod transport;
mod input;
mod web;
mod cursor_overlay;
mod display;

// WebRTC + GStreamer modules (feature-gated)
#[cfg(feature = "webrtc-streaming")]
mod gstreamer;

#[cfg(feature = "webrtc-streaming")]
mod webrtc;

use args::Args;
use clap::Parser;
use config::Config;
use capture::X11Capturer;
use encode::{Encoder, EncoderConfig};
use audio::{run_audio_capture, AudioConfig as RuntimeAudioConfig};
use input::{InputConfig, InputEventData, InputInjector};
use display::{DisplayManager, DisplayManagerConfig};
use base64::Engine;
use image::ImageEncoder;
use log::{info, error, warn, debug};
use std::ffi::CString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::signal;
use tokio::task;
use tokio::sync::mpsc;
use x11rb::xcb_ffi::XCBConnection;
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{ConnectionExt as XFixesConnectionExt, CursorNotify, CursorNotifyMask};
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt, CreateGCAux, Rectangle};

#[cfg(feature = "webrtc-streaming")]
use gstreamer::PipelineConfig;

#[cfg(feature = "webrtc-streaming")]
use webrtc::SessionManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize logging
    env_logger::init_from_env(
        env_logger::Env::default()
            .filter_or("SELKIES_LOG", if args.verbose { "debug" } else { "info" })
    );

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
    if let Some(backend) = args.x11_backend {
        config.display.x11_backend = backend;
    }
    if let Some(range_str) = args.x11_display_range {
        if let Some((start, end)) = parse_display_range(&range_str) {
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

    if let Some(port) = args.port {
        info!("Overriding WebSocket port to {}", port);
        config.websocket.port = port;
    }
    if let Some(port) = args.http_port {
        info!("Overriding HTTP port to {}", port);
        config.http.port = port;
    }

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Invalid configuration: {}", e);
        return Err(e.into());
    }

    // Initialize GStreamer if WebRTC is enabled
    #[cfg(feature = "webrtc-streaming")]
    if config.webrtc.enabled {
        info!("Initializing GStreamer for WebRTC streaming...");
        if let Err(e) = gstreamer::init() {
            error!("Failed to initialize GStreamer: {}", e);
            if cfg!(feature = "websocket-legacy") {
                warn!("Falling back to WebSocket-only mode");
                // Continue without WebRTC
            } else {
                return Err(e.into());
            }
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
    let state = Arc::new(web::SharedState::new(config.clone(), ui_config, input_tx.clone()));
    let running = Arc::new(AtomicBool::new(true));

    // Create WebRTC session manager if enabled
    #[cfg(feature = "webrtc-streaming")]
    let session_manager = if config.webrtc.enabled {
        info!("Creating WebRTC session manager...");
        let manager = Arc::new(SessionManager::new(
            config.webrtc.clone(),
            input_tx.clone(),
            10, // max 10 concurrent sessions
        ));
        info!("WebRTC session manager created");
        Some(manager)
    } else {
        None
    };

    #[cfg(not(feature = "webrtc-streaming"))]
    let session_manager: Option<Arc<()>> = None;

    // Start WebSocket server
    let ws_server = transport::WebSocketServer::new(
        config.websocket.host.clone(),
        config.websocket.port,
        state.clone(),
    );

    let mut server_handle = task::spawn(async move {
        if let Err(e) = ws_server.run().await {
            error!("WebSocket server error: {}", e);
        }
    });

    // Start GStreamer pipeline for WebRTC (if enabled)
    #[cfg(feature = "webrtc-streaming")]
    let gst_handle = if config.webrtc.enabled && session_manager.is_some() {
        let gst_state = state.clone();
        let gst_running = running.clone();
        let pipeline_config = PipelineConfig::from(&config.webrtc);

        Some(task::spawn_blocking(move || {
            info!("Starting GStreamer pipeline...");

            let pipeline = match gstreamer::VideoPipeline::new(pipeline_config) {
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

            info!("GStreamer pipeline started, streaming RTP packets...");
            let mut packet_count = 0u64;

            while gst_running.load(Ordering::Relaxed) {
                // Check for keyframe request
                if gst_state.take_keyframe_request() {
                    pipeline.request_keyframe();
                    debug!("Keyframe requested");
                }

                // Pull RTP packet from pipeline
                if let Some(sample) = pipeline.try_pull_sample() {
                    if let Some(buffer) = sample.buffer() {
                        if let Ok(map) = buffer.map_readable() {
                            let packet = map.as_slice().to_vec();

                            // Broadcast to all WebRTC sessions
                            gst_state.broadcast_rtp(packet);

                            packet_count += 1;
                            if packet_count % 1000 == 0 {
                                debug!("Sent {} RTP packets", packet_count);
                            }
                        }
                    }
                } else {
                    // No packet available, sleep briefly
                    std::thread::sleep(Duration::from_millis(1));
                }
            }

            info!("GStreamer pipeline stopped");
            let _ = pipeline.stop();
            Ok::<(), String>(())
        }))
    } else {
        None
    };

    #[cfg(not(feature = "webrtc-streaming"))]
    let gst_handle: Option<tokio::task::JoinHandle<Result<(), String>>> = None;

    // Start capture and encoding loop (for WebSocket/legacy mode)
    let capture_state = state.clone();
    let display = config.display.display.clone();
    let target_fps = config.encoding.target_fps.max(1);
    let encoder_config = EncoderConfig {
        quality: config.encoding.jpeg_quality,
        stripe_height: config.encoding.stripe_height,
        subsample: 1,
    };
    let capture_running = running.clone();
    let cursor_hidden = Arc::new(AtomicBool::new(false));
    let cursor_hidden_capture = Arc::clone(&cursor_hidden);
    let capture_display_label = config.display.display.clone();
    let mut capture_handle = task::spawn_blocking(move || {
        let frame_interval = Duration::from_millis(1000 / target_fps as u64);
        while capture_running.load(Ordering::Relaxed) {
            let display_cstr = match CString::new(display.as_str()) {
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
            let screen = &conn.setup().roots[screen_num as usize];
            let root_window = screen.root;
            if !cursor_hidden_capture.load(Ordering::Relaxed) {
                if hide_cursor(conn.as_ref(), root_window) {
                    cursor_hidden_capture.store(true, Ordering::Relaxed);
                    info!("Cursor hidden on display {}", capture_display_label);
                }
            }

            let mut capturer = match X11Capturer::new(conn.clone(), screen_num) {
                Ok(value) => value,
                Err(e) => {
                    error!("Capturer init failed: {}", e);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let (cap_width, cap_height) = capturer.dimensions();
            info!("Capturer initialized: {}x{}", cap_width, cap_height);
            capture_state.set_display_size(cap_width, cap_height);
            let mut encoder = match Encoder::new(encoder_config.clone()) {
                Ok(value) => value,
                Err(e) => {
                    error!("Encoder init failed: {}", e);
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };

            let mut last_frame_ts: Option<std::time::Instant> = None;
            while capture_running.load(Ordering::Relaxed) {
                if capture_state.take_refresh_request() {
                    encoder.force_refresh();
                }
                let frame_start = std::time::Instant::now();

                match capturer.capture() {
                    Ok(frame) => {
                        match encoder.encode_frame(&frame) {
                            Ok(mut stripes) => {
                                if frame.is_dirty && !stripes.is_empty() {
                                    let frame_id = (frame.sequence & 0xFFFF) as u16;
                                    for stripe in stripes.iter_mut() {
                                        stripe.frame_id = frame_id;
                                    }
                                    if frame.sequence % 30 == 0 {
                                        debug!("Sending frame {} with {} stripes", frame_id, stripes.len());
                                    }
                                    let _ = capture_state.frame_sender.send(stripes);
                                }
                                let fps = match last_frame_ts.replace(frame.timestamp) {
                                    Some(prev) => {
                                        let dt = frame.timestamp.duration_since(prev).as_secs_f64();
                                        if dt > 0.0 { 1.0 / dt } else { target_fps as f64 }
                                    }
                                    None => target_fps as f64,
                                };
                                capture_state.update_capture_stats(frame.data.len(), fps);
                            }
                            Err(e) => error!("Encode error: {}", e),
                        }
                    }
                    Err(e) => {
                        error!("Capture error: {}", e);
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }

                let elapsed = frame_start.elapsed();
                if elapsed < frame_interval {
                    std::thread::sleep(frame_interval - elapsed);
                }
            }
        }
        Ok::<(), String>(())
    });

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
                match input_rx.try_recv() {
                    Ok(event) => {
                        if let Err(e) = injector.process_event(event) {
                            error!("Input event failed: {}", e);
                        }
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
        }

        Ok::<(), String>(())
    });

    // Start audio streaming loop
    let audio_running = running.clone();
    let audio_sender = state.audio_sender.clone();
    let audio_config = RuntimeAudioConfig {
        sample_rate: config.audio.sample_rate,
        channels: config.audio.channels,
        bitrate: config.audio.bitrate,
    };
    let mut audio_handle = if config.audio.enabled {
        Some(task::spawn_blocking(move || {
            if let Err(e) = run_audio_capture(audio_config, audio_sender, audio_running) {
                error!("Audio capture error: {}", e);
            }
        }))
    } else {
        None
    };

    // Start cursor tracking loop
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
            if let Err(e) = conn.xfixes_select_cursor_input(root, CursorNotifyMask::DISPLAY_CURSOR) {
                warn!("Cursor tracking: Failed to select cursor input events: {:?}", e);
            }
            let mut last = (i16::MIN, i16::MIN);
            let mut last_serial = u32::MAX; // Initialize to MAX so first cursor is always sent
            let mut cursor_msg_count = 0u64;
            let mut first_message_sent = false;
            let mut cursor_override: Option<String> = None;
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
                                        if !first_message_sent {
                                            info!("Cursor tracking: Failed to get cursor image reply: {:?}", e);
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
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Ok::<(), String>(())
    }));

    // Start WebRTC session cleanup task (if enabled)
    #[cfg(feature = "webrtc-streaming")]
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

    #[cfg(not(feature = "webrtc-streaming"))]
    let cleanup_handle: Option<tokio::task::JoinHandle<()>> = None;

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
    #[cfg(feature = "webrtc-streaming")]
    let http_server = web::run_http_server_with_webrtc(
        config.http.port,
        state.clone(),
        session_manager.clone(),
    );

    #[cfg(not(feature = "webrtc-streaming"))]
    let http_server = web::run_http_server(config.http.port, state.clone());

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

    // Build select branches based on what's enabled
    #[cfg(feature = "webrtc-streaming")]
    let _has_gst = gst_handle.is_some();
    #[cfg(not(feature = "webrtc-streaming"))]
    let has_gst = false;

    #[cfg(feature = "webrtc-streaming")]
    let _has_cleanup = cleanup_handle.is_some();
    #[cfg(not(feature = "webrtc-streaming"))]
    let has_cleanup = false;

    if let Some(handle) = audio_handle.as_mut() {
        tokio::select! {
            _ = shutdown => {
                info!("Initiating graceful shutdown...");
            }
            result = &mut server_handle => {
                log_async_task_result("WebSocket server", result);
            }
            result = &mut capture_handle => {
                log_blocking_task_result("Capture loop", result);
            }
            result = &mut input_handle => {
                log_blocking_task_result("Input loop", result);
            }
            result = &mut stats_handle => {
                log_blocking_task_result("Stats loop", result);
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
            result = &mut server_handle => {
                log_async_task_result("WebSocket server", result);
            }
            result = &mut capture_handle => {
                log_blocking_task_result("Capture loop", result);
            }
            result = &mut input_handle => {
                log_blocking_task_result("Input loop", result);
            }
            result = &mut stats_handle => {
                log_blocking_task_result("Stats loop", result);
            }
            result = &mut http_handle => {
                log_async_task_result("HTTP server", result);
            }
        }
    }

    // Cleanup
    running.store(false, Ordering::Relaxed);

    info!("Stopping all tasks...");

    // Stop WebSocket server
    if !server_handle.is_finished() {
        server_handle.abort();
        let _ = server_handle.await;
    }

    // Stop HTTP server
    if !http_handle.is_finished() {
        http_handle.abort();
        let _ = http_handle.await;
    }

    // Stop GStreamer pipeline
    #[cfg(feature = "webrtc-streaming")]
    if let Some(handle) = gst_handle {
        if !handle.is_finished() {
            info!("Stopping GStreamer pipeline...");
            handle.abort();
            let _ = handle.await;
        }
    }

    // Stop WebRTC cleanup task
    #[cfg(feature = "webrtc-streaming")]
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

    // Stop capture loop
    if !capture_handle.is_finished() {
        capture_handle.abort();
        let _ = capture_handle.await;
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
