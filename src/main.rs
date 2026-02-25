//! iVnc - Main entry point
//!
//! Wayland compositor + WebRTC streaming using smithay and GStreamer.

mod args;
mod config;
mod audio;
mod file_upload;
mod clipboard;
mod system_clipboard;
mod runtime_settings;
mod transport;
mod input;
mod web;
mod compositor;
mod gstreamer;
mod webrtc;

use args::Args;
use base64::Engine;
use clap::Parser;
use ::gstreamer as gst;
use config::Config;
use audio::{run_audio_capture, AudioConfig as RuntimeAudioConfig};
use compositor::{Compositor, HeadlessBackend};
use input::{InputEvent, InputEventData};
use log::{info, error, warn};
use smithay::reexports::wayland_server::Resource;
use std::env;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use gstreamer::PipelineConfig;
use webrtc::SessionManager;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Look up a .desktop file whose StartupWMClass matches the given app_id,
/// and return its Name= value. Returns None if no match found.
fn resolve_display_name(app_id: &str) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }
    let data_home = env::var("XDG_DATA_HOME")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".local/share")))?;
    let apps_dir = data_home.join("applications");
    let entries = std::fs::read_dir(&apps_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let mut name = None;
        let mut wm_class = None;
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("Name=") {
                name = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("StartupWMClass=") {
                wm_class = Some(v.to_string());
            }
        }
        if wm_class.as_deref() == Some(app_id) {
            return name;
        }
    }
    None
}

/// Check that required shared libraries are present on the system.
/// Prints friendly install instructions and exits if any are missing.
fn check_runtime_deps() {
    let deps: &[(&str, &str)] = &[
        ("libgstreamer-1.0.so.0", "libgstreamer1.0-0"),
        ("libgstapp-1.0.so.0", "libgstreamer-plugins-base1.0-0"),
        ("libpixman-1.so.0", "libpixman-1-0"),
        ("libxkbcommon.so.0", "libxkbcommon0"),
        #[cfg(feature = "pulseaudio")]
        ("libpulse-simple.so.0", "libpulse0"),
        #[cfg(any(feature = "pulseaudio", feature = "audio"))]
        ("libopus.so.0", "libopus0"),
    ];

    let mut missing = Vec::new();
    for &(soname, pkg) in deps {
        let cstr = std::ffi::CString::new(soname).unwrap();
        let handle = unsafe { libc::dlopen(cstr.as_ptr(), libc::RTLD_LAZY) };
        if handle.is_null() {
            missing.push((soname, pkg));
        } else {
            unsafe { libc::dlclose(handle); }
        }
    }

    if !missing.is_empty() {
        eprintln!("ERROR: Missing runtime libraries:");
        for (soname, pkg) in &missing {
            eprintln!("  {} (package: {})", soname, pkg);
        }
        let pkgs: Vec<&str> = missing.iter().map(|(_, p)| *p).collect();
        eprintln!("\nInstall with:\n  apt-get install {}", pkgs.join(" "));
        std::process::exit(1);
    }

    // Check GStreamer plugins
    if gst::init().is_err() {
        eprintln!("ERROR: Failed to initialize GStreamer");
        std::process::exit(1);
    }

    let gst_plugins: &[(&str, &str)] = &[
        ("videoconvert", "gstreamer1.0-plugins-base"),
        ("appsrc", "gstreamer1.0-plugins-base"),
        ("rtph264pay", "gstreamer1.0-plugins-good"),
        ("rtpvp8pay", "gstreamer1.0-plugins-good"),
        ("openh264enc", "gstreamer1.0-plugins-bad"),
        ("ximagesrc", "gstreamer1.0-x"),
    ];

    let mut missing_plugins = Vec::new();
    for &(element, pkg) in gst_plugins {
        if gst::ElementFactory::find(element).is_none() {
            missing_plugins.push((element, pkg));
        }
    }

    if !missing_plugins.is_empty() {
        eprintln!("WARNING: Missing GStreamer plugins:");
        for (element, pkg) in &missing_plugins {
            eprintln!("  {} (package: {})", element, pkg);
        }
        let mut pkgs: Vec<&str> = missing_plugins.iter().map(|(_, p)| *p).collect();
        pkgs.sort_unstable();
        pkgs.dedup();
        eprintln!("\nInstall with:\n  apt-get install {}", pkgs.join(" "));
    }
}

fn main() {
    check_runtime_deps();

    let args = Args::parse();

    let log_level = if args.verbose { "debug" } else { "info" };
    env_logger::Builder::new()
        .parse_filters(&format!(
            "ivnc={},smithay={},str0m=warn,webrtc=warn,webrtc_ice=warn",
            log_level, log_level
        ))
        .init();

    info!("ivnc v{} starting", env!("CARGO_PKG_VERSION"));

    let mut config = match args.load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    apply_cli_overrides(&mut config, &args);

    if let Err(e) = config.validate() {
        eprintln!("Invalid configuration: {}", e);
        error!("Invalid configuration: {}", e);
        std::process::exit(1);
    }
    let width = config.display.width;
    let height = config.display.height;
    info!("Display: {}x{}", width, height);
    info!("Codec: {:?}, Bitrate: {} kbps", config.webrtc.video_codec, config.webrtc.video_bitrate);

    let runtime_settings = Arc::new(runtime_settings::RuntimeSettings::new(&config));
    let (input_tx, input_rx) = mpsc::unbounded_channel::<InputEventData>();
    let ui_config = config::ui::UiConfig::from_env(&config);

    let shared_state = Arc::new(web::SharedState::new(
        config.clone(), ui_config, input_tx.clone(), runtime_settings.clone(),
    ));

    if let Err(e) = run(config, shared_state, runtime_settings, input_rx, width, height) {
        eprintln!("Fatal error: {}", e);
        error!("Fatal error: {}", e);
        std::process::exit(1);
    }
}

fn run(
    config: Config,
    shared_state: Arc<web::SharedState>,
    runtime_settings: Arc<runtime_settings::RuntimeSettings>,
    mut input_rx: mpsc::UnboundedReceiver<InputEventData>,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let running = Arc::new(AtomicBool::new(true));

    use smithay::reexports::calloop::EventLoop;
    use smithay::reexports::wayland_server::Display;

    // All env::set_var calls below happen before any threads are spawned
    // (tokio runtime is created later). This is important because set_var
    // is not thread-safe.

    // Ensure XDG_RUNTIME_DIR is set (required for Wayland socket)
    if env::var("XDG_RUNTIME_DIR").is_err() {
        let dir = format!("/run/user/{}", unsafe { libc::getuid() });
        std::fs::create_dir_all(&dir).ok();
        env::set_var("XDG_RUNTIME_DIR", &dir);
        info!("Set XDG_RUNTIME_DIR={}", dir);
    }

    // Clean up wayland sockets: kill non-ivnc processes occupying them
    if let Ok(xdg_dir) = env::var("XDG_RUNTIME_DIR") {
        for i in 0..3 {
            let sock = format!("{}/wayland-{}", xdg_dir, i);
            let lock = format!("{}.lock", sock);
            if !std::path::Path::new(&sock).exists() {
                continue;
            }
            if std::os::unix::net::UnixStream::connect(&sock).is_err() {
                // Stale socket, just remove
                std::fs::remove_file(&sock).ok();
                std::fs::remove_file(&lock).ok();
                continue;
            }
            // Socket is alive — find and kill the non-ivnc listener
            if let Ok(output) = std::process::Command::new("fuser").arg(&sock).output() {
                let pids_str = String::from_utf8_lossy(&output.stdout);
                for token in pids_str.split_whitespace() {
                    let pid: i32 = match token.trim().parse() {
                        Ok(p) if p > 1 => p,
                        _ => continue,
                    };
                    // Check if it's our own process
                    if pid == std::process::id() as i32 {
                        continue;
                    }
                    // Read process name
                    let comm = std::fs::read_to_string(format!("/proc/{}/comm", pid))
                        .unwrap_or_default();
                    if comm.trim() == "ivnc" {
                        continue;
                    }
                    warn!("Killing non-ivnc process {} ({}) occupying {}", pid, comm.trim(), sock);
                    unsafe { libc::kill(pid, libc::SIGKILL); }
                }
                std::thread::sleep(Duration::from_millis(200));
                std::fs::remove_file(&sock).ok();
                std::fs::remove_file(&lock).ok();
            }
        }
    }

    let mut event_loop: EventLoop<Compositor> = EventLoop::try_new()?;
    let display: Display<Compositor> = Display::new()?;
    let mut comp = Compositor::new(&mut event_loop, display);

    let mut backend = HeadlessBackend::new(width, height)?;
    let _output_global = backend.output().create_global::<Compositor>(&comp.display_handle);
    comp.space.map_output(backend.output(), (0, 0));

    let socket_name = comp.socket_name.clone();
    env::set_var("WAYLAND_DISPLAY", &socket_name);
    // Disable GTK3 CSD via gtk3-nocsd (process-level env only)
    env::set_var("GTK_CSD", "0");
    let nocsd_lib = "/lib/x86_64-linux-gnu/libgtk3-nocsd.so.0";
    if std::path::Path::new(nocsd_lib).exists() {
        let existing = env::var("LD_PRELOAD").unwrap_or_default();
        if existing.is_empty() {
            env::set_var("LD_PRELOAD", nocsd_lib);
        } else {
            env::set_var("LD_PRELOAD", format!("{}:{}", existing, nocsd_lib));
        }
        info!("gtk3-nocsd enabled via LD_PRELOAD");
    }
    // Set up GTK CSS to hide headerbars on fullscreen windows only.
    // Dialogs are excluded so their controls (Open/Cancel buttons) stay visible.
    setup_gtk_css_env();
    info!("Wayland socket: {:?}", socket_name);

    // GStreamer pipeline
    let pipeline_config = PipelineConfig {
        width, height,
        framerate: config.encoding.target_fps,
        codec: config.webrtc.video_codec,
        bitrate: config.webrtc.video_bitrate,
        hardware_encoder: config.webrtc.hardware_encoder,
        keyframe_interval: config.webrtc.keyframe_interval,
        latency_ms: config.webrtc.pipeline_latency_ms,
    };
    let mut pipeline = gstreamer::VideoPipeline::new(pipeline_config)?;
    pipeline.start()?;
    info!("GStreamer pipeline started (encoder: {})", pipeline.encoder_name());

    // Tokio runtime for async services
    let tokio_rt = tokio::runtime::Runtime::new()?;
    {
        let st = shared_state.clone();
        let r = running.clone();
        let c = config.clone();
        let rs = runtime_settings.clone();
        tokio_rt.spawn(async move {
            if let Err(e) = run_async_services(c, st, rs, r).await {
                error!("Async services error: {}", e);
            }
        });
    }

    // Audio capture thread
    if config.audio.enabled {
        info!("Starting audio capture thread (rate={} ch={} bitrate={})",
            config.audio.sample_rate, config.audio.channels, config.audio.bitrate);
        let r = running.clone();
        let ac = config.audio.clone();
        let (audio_tx, mut audio_rx) = mpsc::unbounded_channel();
        let st = shared_state.clone();
        tokio_rt.spawn(async move {
            while let Some(pkt) = audio_rx.recv().await {
                st.broadcast_audio(pkt);
            }
        });
        std::thread::Builder::new().name("audio-capture".into()).spawn(move || {
            info!("Audio capture thread started");
            let rt_audio = RuntimeAudioConfig {
                sample_rate: ac.sample_rate, channels: ac.channels, bitrate: ac.bitrate,
            };
            match run_audio_capture(rt_audio, audio_tx, r) {
                Ok(()) => info!("Audio capture thread exited normally"),
                Err(e) => warn!("Audio capture ended with error: {}", e),
            }
        })?;
    } else {
        info!("Audio capture disabled in config");
    }

    // Main compositor loop
    let target_fps = shared_state.config.encoding.target_fps.max(1);
    let frame_duration = Duration::from_micros(1_000_000 / target_fps as u64);
    let mut last_frame = Instant::now();
    let mut last_stats = Instant::now();
    let mut frame_count: u64 = 0;
    let mut byte_count: u64 = 0;

    let mut render_frames: u64 = 0;
    let mut rtp_packets: u64 = 0;
    let mut prev_window_count: usize = 0;
    let mut keyframe_buf: Vec<Vec<u8>> = Vec::new();
    let mut in_keyframe = false;
    let mut rtp_frame_buf: Vec<Vec<u8>> = Vec::new();
    let mut prev_rtp_ts: Option<u32> = None;
    let mut last_rtp_sample: Option<Instant> = None;
    let mut last_render = Instant::now();
    let mut prev_button_mask: u32 = 0;
    let (disp_w, disp_h) = shared_state.display_size();
    let mut prev_cursor_pos: (f64, f64) = (disp_w as f64 / 2.0, disp_h as f64 / 2.0);
    let mut prev_cursor_name: String = "default".to_string();
    let mut prev_taskbar_json: String = String::new();
    let mut prev_dc_open_count: u64 = 0;
    // Non-blocking clipboard pipe read state
    let mut clipboard_pipe: Option<std::fs::File> = None;
    let mut clipboard_pipe_buf: Vec<u8> = Vec::new();

    info!("Compositor loop starting at {} fps", target_fps);

    while running.load(Ordering::Relaxed) {
        event_loop.dispatch(Some(Duration::from_millis(1)), &mut comp)?;
        comp.space.refresh();
        comp.popups.cleanup();
        comp.display_handle.flush_clients().ok();

        // Deferred clipboard read: new_selection saved the mime type but couldn't
        // call request_data_device_client_selection because smithay hadn't updated
        // the seat's selection yet. Now after dispatch() it's safe to request.
        if let Some(mime) = comp.clipboard_pending_mime.take() {
            use std::os::fd::{AsRawFd, FromRawFd};
            use smithay::wayland::selection::data_device::request_data_device_client_selection;

            let mut fds = [0i32; 2];
            if unsafe { libc::pipe(fds.as_mut_ptr()) } == 0 {
                let read_fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[0]) };
                let write_fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fds[1]) };
                // Set read end to non-blocking
                unsafe {
                    let flags = libc::fcntl(read_fd.as_raw_fd(), libc::F_GETFL);
                    if flags >= 0 {
                        libc::fcntl(read_fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                }
                info!("Deferred clipboard: requesting client data for mime={}", mime);
                if request_data_device_client_selection::<Compositor>(&comp.seat, mime, write_fd).is_ok() {
                    comp.clipboard_read_fd = Some(read_fd);
                    // Flush immediately so the client receives the fd and can write data
                    comp.display_handle.flush_clients().ok();
                } else {
                    warn!("Deferred clipboard: request_data_device_client_selection failed");
                }
            } else {
                warn!("Deferred clipboard: pipe() failed");
            }
        }

        // Browser clipboard → remote compositor (drain all pending items).
        // Process BEFORE input events so that when Ctrl+V arrives, the
        // clipboard selection is already set and the app can read it.
        {
            let mut rx = shared_state.clipboard_incoming_rx.lock().unwrap();
            while let Ok(b64) = rx.try_recv() {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                    if let Ok(text) = String::from_utf8(bytes) {
                        use smithay::wayland::selection::data_device::set_data_device_selection;
                        comp.pending_paste = Some(text.clone());
                        let dh = comp.display_handle.clone();
                        let seat = comp.seat.clone();
                        set_data_device_selection(
                            &dh, &seat,
                            vec!["text/plain;charset=utf-8".into(), "text/plain".into(), "UTF8_STRING".into()],
                            (),
                        );
                        // Suppress client clipboard re-assertions for a short window.
                        // The focused client (e.g. Chromium) will re-assert its own
                        // wl_data_source with stale content in response to our selection change.
                        comp.clipboard_suppress_until = Some(Instant::now() + Duration::from_millis(500));
                        info!("Clipboard from browser: {} bytes", text.len());
                    }
                }
            }
        }
        comp.display_handle.flush_clients().ok();

        drain_input_events(
            &mut input_rx,
            &mut comp,
            &shared_state,
            &mut prev_button_mask,
            &mut prev_cursor_pos,
        );
        comp.display_handle.flush_clients().ok(); // flush injected input events immediately

        // Read clipboard from Wayland client (remote → browser).
        // The pipe read fd is non-blocking so we accumulate data across
        // loop iterations without deadlocking the compositor.
        if let Some(fd) = comp.clipboard_read_fd.take() {
            clipboard_pipe_buf.clear();
            clipboard_pipe = Some(std::fs::File::from(fd));
        }
        if let Some(ref mut file) = clipboard_pipe {
            let mut tmp = [0u8; 4096];
            loop {
                match file.read(&mut tmp) {
                    Ok(0) => {
                        // EOF — client closed write end, data is complete
                        if !clipboard_pipe_buf.is_empty() {
                            if let Ok(text) = String::from_utf8(clipboard_pipe_buf.clone()) {
                                let encoded = base64::engine::general_purpose::STANDARD.encode(&text);
                                let msg = format!("clipboard,{}", encoded);
                                info!("Clipboard from remote app: {} bytes", text.len());
                                shared_state.send_text(msg);
                                info!("Clipboard broadcast to remote");
                            }
                        }
                        clipboard_pipe_buf.clear();
                        clipboard_pipe = None;
                        break;
                    }
                    Ok(n) => {
                        clipboard_pipe_buf.extend_from_slice(&tmp[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No more data available yet, try again next iteration
                        break;
                    }
                    Err(_) => {
                        // Pipe error, discard
                        clipboard_pipe_buf.clear();
                        clipboard_pipe = None;
                        break;
                    }
                }
            }
        }

        // Broadcast cursor changes to frontend
        let cursor_name = match &comp.cursor_status {
            smithay::input::pointer::CursorImageStatus::Hidden => "none".to_string(),
            smithay::input::pointer::CursorImageStatus::Named(icon) => icon.name().to_string(),
            _ => "default".to_string(),
        };
        if cursor_name != prev_cursor_name {
            info!("Cursor changed: {} -> {}", prev_cursor_name, cursor_name);
            let msg = format!("cursor,{{\"override\":\"{}\"}}", cursor_name);
            shared_state.send_text(msg);
            prev_cursor_name = cursor_name;
        }

        // Detect window changes and request keyframe so browsers can decode the new content
        let cur_window_count = comp.space.elements().count();
        if cur_window_count != prev_window_count {
            info!("Window count changed: {} -> {}", prev_window_count, cur_window_count);
            prev_window_count = cur_window_count;
            backend.reset_damage();
            pipeline.request_keyframe();
            comp.needs_redraw = true;
            comp.taskbar_dirty = true;
        }

        // Force taskbar resend when a new DataChannel opens
        // (receiver_count increases at subscribe time, before the DC is ready,
        //  so we use the datachannel_open_count which bumps on ChannelOpen)
        let cur_dc_open = shared_state.datachannel_open_count.load(Ordering::Relaxed);
        if cur_dc_open > prev_dc_open_count {
            prev_taskbar_json.clear();
            comp.taskbar_dirty = true;
        }
        prev_dc_open_count = cur_dc_open;

        // Broadcast taskbar window list to frontend when dirty
        if comp.taskbar_dirty {
            comp.taskbar_dirty = false;
            let focused_wl = comp.seat.get_keyboard()
                .and_then(|kb| kb.current_focus());
            let mut windows_json = Vec::new();
            for (idx, wl_surface) in comp.window_registry.iter().enumerate() {
                // Skip if window not in space anymore (being destroyed)
                if comp.space.elements()
                    .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                    .is_none() {
                    continue;
                };
                let is_focused = focused_wl.as_ref()
                    .map(|f| f.id() == wl_surface.id())
                    .unwrap_or(false);
                let (title, app_id) = smithay::wayland::compositor::with_states(wl_surface, |states| {
                    let data = states.data_map
                        .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap();
                    (
                        data.title.clone().unwrap_or_default(),
                        data.app_id.clone().unwrap_or_default(),
                    )
                });
                if is_focused {
                    comp.focused_surface_id = Some(idx as u32);
                }
                windows_json.push(serde_json::json!({
                    "id": idx,
                    "title": title,
                    "app_id": app_id,
                    "display_name": resolve_display_name(&app_id),
                    "focused": is_focused,
                }));
            }
            let json = serde_json::json!({ "windows": windows_json }).to_string();
            if json != prev_taskbar_json {
                prev_taskbar_json = json.clone();
                let msg = format!("taskbar,{}", json);
                info!("Taskbar broadcast: {}", msg);
                shared_state.send_text(msg);
            }
        }

        if let Some((w, h)) = shared_state.take_pending_resize() {
            if let Err(e) = backend.resize(w, h) {
                warn!("Resize failed: {}", e);
            } else {
                shared_state.set_display_size(w, h);

                // Re-configure all non-dialog toplevel windows to the new output size
                let new_size: smithay::utils::Size<i32, smithay::utils::Logical> =
                    (w as i32, h as i32).into();
                for window in comp.space.elements() {
                    let toplevel = window.toplevel().unwrap();
                    let surface_id = toplevel.wl_surface().id().protocol_id();
                    if comp.dialog_surfaces.contains(&surface_id) {
                        continue;
                    }
                    toplevel.with_pending_state(|state| {
                        state.size = Some(new_size);
                    });
                    toplevel.send_pending_configure();
                }

                // Rebuild pipeline with new dimensions
                info!("Rebuilding GStreamer pipeline for {}x{}", w, h);
                let _ = pipeline.stop();
                let new_config = PipelineConfig {
                    width: w, height: h,
                    framerate: config.encoding.target_fps,
                    codec: config.webrtc.video_codec,
                    bitrate: config.webrtc.video_bitrate,
                    hardware_encoder: config.webrtc.hardware_encoder,
                    keyframe_interval: config.webrtc.keyframe_interval,
                    latency_ms: config.webrtc.pipeline_latency_ms,
                };
                match gstreamer::VideoPipeline::new(new_config) {
                    Ok(new_pipeline) => {
                        if let Err(e) = new_pipeline.start() {
                            error!("Failed to start new pipeline: {}", e);
                        } else {
                            pipeline = new_pipeline;
                            info!("Pipeline rebuilt for {}x{}", w, h);
                        }
                    }
                    Err(e) => error!("Failed to create new pipeline: {}", e),
                }
            }
        }

        apply_runtime_settings(&runtime_settings, &pipeline);

        // Send frame callbacks BEFORE sleep so clients have the full
        // frame period to prepare and commit their next buffer.
        backend.send_frame_callbacks(&comp);
        comp.display_handle.flush_clients().ok();

        // Frame timing — clients are working in parallel during this sleep
        let elapsed = last_frame.elapsed();
        if elapsed < frame_duration {
            std::thread::sleep(frame_duration - elapsed);
        }
        last_frame = Instant::now();

        // Quick dispatch to pick up commits that arrived during sleep
        event_loop.dispatch(Some(Duration::ZERO), &mut comp)?;
        comp.display_handle.flush_clients().ok();

        // Render + encode if any client committed new content
        // Also force periodic renders when sessions are active to ensure
        // the browser always has decodable video frames.
        let has_sessions = shared_state.rtp_receiver_count() > 0;
        if !comp.needs_redraw && has_sessions && last_render.elapsed() >= Duration::from_secs(1) {
            comp.needs_redraw = true;
        }
        if comp.needs_redraw {
            comp.needs_redraw = false;
            match backend.render_frame(&mut comp) {
                Some(pixels) => {
                    render_frames += 1;
                    last_render = Instant::now();
                    if let Err(e) = pipeline.push_frame(&pixels) {
                        warn!("Failed to push frame: {}", e);
                        continue;
                    }
                    frame_count += 1;
                    byte_count += pixels.len() as u64;
                }
                None => {
                    warn!("render_frame returned None (windows={})", comp.space.elements().count());
                }
            }
        }

        pull_and_broadcast_rtp(
            &pipeline,
            &shared_state,
            &mut rtp_packets,
            &mut keyframe_buf,
            &mut in_keyframe,
            &mut rtp_frame_buf,
            &mut prev_rtp_ts,
            &mut last_rtp_sample,
        );

        if shared_state.take_keyframe_request() {
            pipeline.request_keyframe();
        }

        if last_stats.elapsed() >= Duration::from_secs(1) {
            let secs = last_stats.elapsed().as_secs_f64();
            let windows = comp.space.elements().count();
            info!(
                "Loop stats: windows={}, rendered={}, pushed={}, rtp_pkts={}, secs={:.1}",
                windows, render_frames, frame_count, rtp_packets, secs
            );
            {
                let mut stats = shared_state.stats.lock().unwrap();
                stats.fps = frame_count as f64 / secs;
                stats.bandwidth = (byte_count as f64 * 8.0 / secs) as u64;
                stats.total_frames += frame_count;
                stats.total_bytes += byte_count;
            }
            shared_state.send_text(
                format!("stats,{}", shared_state.stats_json()),
            );
            // Re-broadcast cursor state so newly connected sessions get it
            shared_state.send_text(
                format!("cursor,{{\"override\":\"{}\"}}", prev_cursor_name),
            );
            render_frames = 0;
            frame_count = 0;
            byte_count = 0;
            rtp_packets = 0;
            last_stats = Instant::now();
        }
    }

    info!("Shutting down...");
    running.store(false, Ordering::SeqCst);
    let _ = pipeline.stop();
    tokio_rt.shutdown_timeout(Duration::from_secs(3));
    info!("ivnc stopped");
    Ok(())
}

fn drain_input_events(
    input_rx: &mut mpsc::UnboundedReceiver<InputEventData>,
    state: &mut Compositor,
    shared: &Arc<web::SharedState>,
    prev_button_mask: &mut u32,
    prev_cursor_pos: &mut (f64, f64),
) {
    use smithay::utils::SERIAL_COUNTER;

    while let Ok(ev) = input_rx.try_recv() {
        let serial = SERIAL_COUNTER.next_serial();
        // Use monotonic clock for Wayland event timestamps (milliseconds).
        // The frontend doesn't send timestamps for keyboard events, so
        // ev.timestamp is 0 — Chromium may discard events with time=0.
        let time = (state.start_time.elapsed().as_millis() & 0xFFFFFFFF) as u32;

        match ev.event_type {
            InputEvent::MouseMove => {
                let (mut x, mut y) = if ev.text == "relative" {
                    (prev_cursor_pos.0 + ev.mouse_x as f64, prev_cursor_pos.1 + ev.mouse_y as f64)
                } else {
                    (ev.mouse_x as f64, ev.mouse_y as f64)
                };
                let (disp_w, disp_h) = shared.display_size();
                x = x.clamp(0.0, disp_w.saturating_sub(1) as f64);
                y = y.clamp(0.0, disp_h.saturating_sub(1) as f64);
                *prev_cursor_pos = (x, y);
                let pos = (x, y).into();
                let under = state.surface_under(pos);
                let ptr = state.seat.get_pointer().unwrap();
                ptr.motion(
                    state, under.clone(),
                    &smithay::input::pointer::MotionEvent { location: pos, serial, time },
                );
                ptr.frame(state);

                // Re-send keyboard focus after the first pointer enter.
                // Chromium's Ozone/Wayland layer ignores keyboard events received
                // before wl_pointer.enter, so we re-send wl_keyboard.enter once
                // the pointer has entered the surface.
                if state.kbd_focus_needs_reenter && under.is_some() {
                    let keyboard = state.seat.get_keyboard().unwrap();
                    if let Some(focus) = keyboard.current_focus() {
                        let reenter_serial = SERIAL_COUNTER.next_serial();
                        info!("Re-sending keyboard focus after first pointer enter");
                        keyboard.set_focus(state, None, reenter_serial);
                        let reenter_serial2 = SERIAL_COUNTER.next_serial();
                        keyboard.set_focus(state, Some(focus), reenter_serial2);
                    }
                    state.kbd_focus_needs_reenter = false;
                }

                // Synthesize button events from buttonMask changes.
                // The frontend sends m,x,y,buttonMask,0 — button state is
                // encoded in the mask, not as separate b,button,pressed messages.
                let new_mask = ev.button_mask;
                if new_mask != *prev_button_mask {
                    info!("ButtonMask changed: {} -> {} at ({},{})", *prev_button_mask, new_mask, ev.mouse_x, ev.mouse_y);
                    let changed = new_mask ^ *prev_button_mask;
                    for bit in 0..5u8 {
                        if changed & (1 << bit) != 0 {
                            let pressed = new_mask & (1 << bit) != 0;
                            let synth = InputEventData {
                                event_type: InputEvent::MouseButton,
                                mouse_x: x as i32,
                                mouse_y: y as i32,
                                mouse_button: bit,
                                button_pressed: pressed,
                                ..Default::default()
                            };
                            let btn_serial = SERIAL_COUNTER.next_serial();
                            inject_button(state, &synth, btn_serial, time);
                        }
                    }
                    *prev_button_mask = new_mask;
                }
            }
            InputEvent::MouseButton => {
                inject_button(state, &ev, serial, time);
            }
            InputEvent::MouseWheel => {
                inject_scroll(state, &ev, time);
            }
            InputEvent::Keyboard => {
                inject_key(state, &ev, serial, time);
            }
            InputEvent::KeyboardReset => {
                // Release all modifier keys to clear stuck state
                let keyboard = state.seat.get_keyboard().unwrap();
                let modifier_keycodes: &[u32] = &[
                    50, 62,   // Shift L/R
                    37, 105,  // Control L/R
                    64, 108,  // Alt L/R
                    133, 134, // Super L/R
                ];
                for &kc in modifier_keycodes {
                    let s = smithay::utils::SERIAL_COUNTER.next_serial();
                    keyboard.input::<(), _>(
                        state,
                        smithay::input::keyboard::Keycode::from(kc),
                        smithay::backend::input::KeyState::Released,
                        s, time,
                        |_, _, _| smithay::input::keyboard::FilterResult::Forward,
                    );
                }
                info!("Keyboard reset: released all modifier keys");
            }
            InputEvent::Ping => {
                shared.send_text("pong".to_string());
            }
            InputEvent::TextInput => {
                inject_text(state, &ev);
            }
            InputEvent::WindowFocus => {
                let target_idx = ev.window_id as usize;
                let wl_surface = state.window_registry.get(target_idx).cloned();
                if let Some(wl_surface) = wl_surface {
                    let window = state.space.elements()
                        .find(|w| w.toplevel().unwrap().wl_surface() == &wl_surface)
                        .cloned();
                    if let Some(window) = window {
                        state.space.raise_element(&window, true);
                        let keyboard = state.seat.get_keyboard().unwrap();
                        keyboard.set_focus(state, Some(wl_surface), serial);
                        state.focused_surface_id = Some(ev.window_id);
                        state.taskbar_dirty = true;
                        state.needs_redraw = true;
                        info!("WindowFocus: switched to window index {}", target_idx);
                    }
                }
            }
            InputEvent::WindowClose => {
                let target_idx = ev.window_id as usize;
                let wl_surface = state.window_registry.get(target_idx).cloned();
                if let Some(wl_surface) = wl_surface {
                    let window = state.space.elements()
                        .find(|w| w.toplevel().unwrap().wl_surface() == &wl_surface)
                        .cloned();
                    if let Some(window) = window {
                        window.toplevel().unwrap().send_close();
                        info!("WindowClose: sent close to window index {}", target_idx);
                        // After close, focus the last window in registry
                        // (the most recently created one that isn't being closed)
                        let last_surface = state.window_registry.iter().enumerate().rev()
                            .find(|(i, _)| *i != target_idx)
                            .map(|(i, s)| (i, s.clone()));
                        if let Some((idx, wl_s)) = last_surface {
                            let next_win = state.space.elements()
                                .find(|w| w.toplevel().unwrap().wl_surface() == &wl_s)
                                .cloned();
                            if let Some(next_win) = next_win {
                                state.space.raise_element(&next_win, true);
                                let keyboard = state.seat.get_keyboard().unwrap();
                                keyboard.set_focus(state, Some(wl_s), serial);
                                state.focused_surface_id = Some(idx as u32);
                                state.needs_redraw = true;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn inject_button(state: &mut Compositor, ev: &InputEventData, serial: smithay::utils::Serial, time: u32) {
    let button = match ev.mouse_button {
        0 => 0x110u32,
        1 => 0x112,
        2 => 0x111,
        b => 0x110 + b as u32,
    };
    let btn_state = if ev.button_pressed {
        smithay::backend::input::ButtonState::Pressed
    } else {
        smithay::backend::input::ButtonState::Released
    };

    // On press, focus the toplevel window under the pointer so it receives keyboard input.
    // We must use the toplevel's wl_surface (not a subsurface from surface_under()),
    // because Chromium routes keyboard events based on which wl_surface has keyboard focus.
    // Using a subsurface would cause Chromium to ignore key events entirely.
    if ev.button_pressed {
        let pos: smithay::utils::Point<f64, smithay::utils::Logical> = (ev.mouse_x as f64, ev.mouse_y as f64).into();
        if let Some((window, _)) = state.space.element_under(pos) {
            if let Some(toplevel) = window.toplevel() {
                let wl_surface = toplevel.wl_surface().clone();
                let keyboard = state.seat.get_keyboard().unwrap();
                keyboard.set_focus(state, Some(wl_surface), serial);
            }
        }
    }

    let ptr = state.seat.get_pointer().unwrap();
    ptr.button(
        state,
        &smithay::input::pointer::ButtonEvent { button, state: btn_state, serial, time },
    );
    ptr.frame(state);
}

fn inject_scroll(state: &mut Compositor, ev: &InputEventData, time: u32) {
    use smithay::backend::input::Axis;
    use smithay::input::pointer::AxisFrame;
    let ptr = state.seat.get_pointer().unwrap();
    let mut frame = AxisFrame::new(time);
    if ev.wheel_delta_y != 0 {
        frame = frame.value(Axis::Vertical, ev.wheel_delta_y as f64);
    }
    if ev.wheel_delta_x != 0 {
        frame = frame.value(Axis::Horizontal, ev.wheel_delta_x as f64);
    }
    ptr.axis(state, frame);
    ptr.frame(state);
}

fn inject_key(state: &mut Compositor, ev: &InputEventData, serial: smithay::utils::Serial, time: u32) {
    use smithay::input::keyboard::{FilterResult, Keycode};
    let keyboard = state.seat.get_keyboard().unwrap();
    let key_state = if ev.key_pressed {
        smithay::backend::input::KeyState::Pressed
    } else {
        smithay::backend::input::KeyState::Released
    };

    // Frontend sends X11 keysyms; smithay expects xkb keycodes (evdev + 8).
    // Use a lookup table for the most common keysyms.
    let keycode = match keysym_to_keycode(ev.keysym) {
        Some(code) => code,
        None => {
            warn!("Unknown keysym 0x{:x}; dropping key event", ev.keysym);
            return;
        }
    };
    let has_focus = keyboard.current_focus().is_some();
    info!("inject_key: keysym=0x{:x} keycode={} pressed={} has_focus={}", ev.keysym, keycode, ev.key_pressed, has_focus);
    keyboard.input::<(), _>(
        state, Keycode::from(keycode), key_state, serial, time,
        |_, _, _| FilterResult::Forward,
    );
}

/// Inject committed text from IME into the focused Wayland client.
/// Uses zwp_text_input_v3 commit_string if the client supports it.
fn inject_text(state: &mut Compositor, ev: &InputEventData) {
    use smithay::wayland::text_input::TextInputSeat;

    if ev.text.is_empty() {
        return;
    }

    let text_input = state.seat.text_input().clone();
    let mut sent = false;

    text_input.with_focused_text_input(|ti, _surface| {
        ti.commit_string(Some(ev.text.clone()));
        ti.done(0);
        sent = true;
    });

    if sent {
        info!("Injected text via text_input protocol: {:?}", ev.text);
    } else {
        // Fallback: set compositor-side clipboard selection, then simulate Ctrl+Shift+V
        info!("No text_input client, using clipboard paste for: {:?}", ev.text);
        use smithay::wayland::selection::data_device::set_data_device_selection;

        state.pending_paste = Some(ev.text.clone());
        let dh = state.display_handle.clone();
        let seat = state.seat.clone();
        set_data_device_selection(
            &dh,
            &seat,
            vec!["text/plain;charset=utf-8".into(), "text/plain".into(), "UTF8_STRING".into()],
            (),
        );

        // Simulate Ctrl+Shift+V (terminal paste shortcut).
        // Use known evdev keycodes directly to avoid keysym mapping gaps.
        let keyboard = state.seat.get_keyboard().unwrap();
        let ctrl_code: u32 = 37;  // Control_L
        let shift_code: u32 = 50; // Shift_L
        let v_code: u32 = 55;     // v

        let s = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(state, smithay::input::keyboard::Keycode::from(ctrl_code),
            smithay::backend::input::KeyState::Pressed, s, 0, |_, _, _| smithay::input::keyboard::FilterResult::Forward);
        let s = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(state, smithay::input::keyboard::Keycode::from(shift_code),
            smithay::backend::input::KeyState::Pressed, s, 0, |_, _, _| smithay::input::keyboard::FilterResult::Forward);
        let s = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(state, smithay::input::keyboard::Keycode::from(v_code),
            smithay::backend::input::KeyState::Pressed, s, 0, |_, _, _| smithay::input::keyboard::FilterResult::Forward);
        let s = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(state, smithay::input::keyboard::Keycode::from(v_code),
            smithay::backend::input::KeyState::Released, s, 0, |_, _, _| smithay::input::keyboard::FilterResult::Forward);
        let s = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(state, smithay::input::keyboard::Keycode::from(shift_code),
            smithay::backend::input::KeyState::Released, s, 0, |_, _, _| smithay::input::keyboard::FilterResult::Forward);
        let s = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.input::<(), _>(state, smithay::input::keyboard::Keycode::from(ctrl_code),
            smithay::backend::input::KeyState::Released, s, 0, |_, _, _| smithay::input::keyboard::FilterResult::Forward);
    }
}

/// Convert X11 keysym to xkb keycode (evdev keycode + 8).
fn keysym_to_keycode(keysym: u32) -> Option<u32> {
    match keysym {
        // Letters (a-z / A-Z)
        0x61 | 0x41 => 38, 0x62 | 0x42 => 56, 0x63 | 0x43 => 54,
        0x64 | 0x44 => 40, 0x65 | 0x45 => 26, 0x66 | 0x46 => 41,
        0x67 | 0x47 => 42, 0x68 | 0x48 => 43, 0x69 | 0x49 => 31,
        0x6a | 0x4a => 44, 0x6b | 0x4b => 45, 0x6c | 0x4c => 46,
        0x6d | 0x4d => 58, 0x6e | 0x4e => 57, 0x6f | 0x4f => 32,
        0x70 | 0x50 => 33, 0x71 | 0x51 => 24, 0x72 | 0x52 => 27,
        0x73 | 0x53 => 39, 0x74 | 0x54 => 28, 0x75 | 0x55 => 30,
        0x76 | 0x56 => 55, 0x77 | 0x57 => 25, 0x78 | 0x58 => 53,
        0x79 | 0x59 => 29, 0x7a | 0x5a => 52,
        // Digits 0-9 and shifted symbols on same keys
        0x30 | 0x29 => 19, 0x31 | 0x21 => 10, 0x32 | 0x40 => 11,
        0x33 | 0x23 => 12, 0x34 | 0x24 => 13, 0x35 | 0x25 => 14,
        0x36 | 0x5e => 15, 0x37 | 0x26 => 16, 0x38 | 0x2a => 17,
        0x39 | 0x28 => 18,
        // Function keys F1-F12
        0xffbe => 67, 0xffbf => 68, 0xffc0 => 69, 0xffc1 => 70,
        0xffc2 => 71, 0xffc3 => 72, 0xffc4 => 73, 0xffc5 => 74,
        0xffc6 => 75, 0xffc7 => 76, 0xffc8 => 95, 0xffc9 => 96,
        // Modifiers
        0xffe1 => 50, 0xffe2 => 62,   // Shift L/R
        0xffe3 => 37, 0xffe4 => 105,  // Control L/R
        0xffe9 => 64, 0xffea => 108,  // Alt L/R
        0xffeb => 133, 0xffec => 134, // Super L/R
        0xffe5 => 66,                 // Caps_Lock
        // Navigation
        0xff0d => 36, 0xff1b => 9, 0xff08 => 22, 0xff09 => 23,
        0x20 => 65, 0xffff => 119, 0xff63 => 118,
        0xff50 => 110, 0xff57 => 115, 0xff55 => 112, 0xff56 => 117,
        // Arrows
        0xff51 => 113, 0xff52 => 111, 0xff53 => 114, 0xff54 => 116,
        // Symbols (key and shifted variant grouped)
        0x2d | 0x5f => 20,  // minus / underscore
        0x3d | 0x2b => 21,  // equal / plus
        0x5b | 0x7b => 34,  // bracketleft / braceleft
        0x5d | 0x7d => 35,  // bracketright / braceright
        0x5c | 0x7c => 51,  // backslash / bar
        0x3b | 0x3a => 47,  // semicolon / colon
        0x27 | 0x22 => 48,  // apostrophe / quotedbl
        0x60 | 0x7e => 49,  // grave / tilde
        0x2c | 0x3c => 59,  // comma / less
        0x2e | 0x3e => 60,  // period / greater
        0x2f | 0x3f => 61,  // slash / question
        // Misc
        0xff13 => 127, 0xff14 => 78, 0xff61 => 107, 0xff7f => 77,
        _ => {
            log::debug!("Unknown keysym 0x{:x}, no keycode mapping", keysym);
            return None;
        }
    }
    .into()
}

/// Check if an RTP packet contains an H.264 keyframe NAL unit.
fn is_h264_keyframe_packet(data: &[u8]) -> bool {
    let hdr_len = webrtc::media_track::rtp_util::header_length(data).unwrap_or(12);
    if data.len() <= hdr_len { return false; }
    let nal_type = data[hdr_len] & 0x1F;
    match nal_type {
        5 | 7 | 8 => true,
        24 => true,
        28 if data.len() > hdr_len + 1 => (data[hdr_len + 1] & 0x1F) == 5,
        _ => false,
    }
}

fn pull_and_broadcast_rtp(
    pipeline: &gstreamer::VideoPipeline,
    shared: &Arc<web::SharedState>,
    rtp_count: &mut u64,
    keyframe_buf: &mut Vec<Vec<u8>>,
    in_keyframe: &mut bool,
    frame_buf: &mut Vec<Vec<u8>>,
    prev_ts: &mut Option<u32>,
    last_sample: &mut Option<Instant>,
) {
    while let Some(sample) = pipeline.try_pull_sample() {
        if let Some(buffer) = sample.buffer() {
            let map = buffer.map_readable().unwrap();
            let data = map.as_slice().to_vec();

            let ts = webrtc::media_track::rtp_util::get_timestamp(&data).unwrap_or(0);

            // When timestamp changes, the previous frame is complete —
            // set marker bit on its last packet and flush.
            if let Some(prev) = *prev_ts {
                if ts != prev && !frame_buf.is_empty() {
                    flush_frame(frame_buf, shared, rtp_count, keyframe_buf, in_keyframe);
                }
            }
            *prev_ts = Some(ts);
            frame_buf.push(data);
            *last_sample = Some(Instant::now());
            let has_marker = frame_buf
                .last()
                .map(|pkt| pkt.len() >= 2 && (pkt[1] & 0x80) != 0)
                .unwrap_or(false);
            if has_marker {
                flush_frame(frame_buf, shared, rtp_count, keyframe_buf, in_keyframe);
            }
        }
    }

    // If no new packets arrived for a short window, flush the buffered frame
    // to avoid stalling when marker bits are missing.
    if !frame_buf.is_empty() {
        if let Some(ts) = last_sample {
            if ts.elapsed() >= Duration::from_millis(50) {
                flush_frame(frame_buf, shared, rtp_count, keyframe_buf, in_keyframe);
            }
        }
    }
}

/// Set the marker bit on the last packet in the frame buffer, then broadcast all packets.
fn flush_frame(
    frame_buf: &mut Vec<Vec<u8>>,
    shared: &Arc<web::SharedState>,
    rtp_count: &mut u64,
    keyframe_buf: &mut Vec<Vec<u8>>,
    in_keyframe: &mut bool,
) {
    // Set marker bit on the last packet of the frame
    if let Some(last) = frame_buf.last_mut() {
        if last.len() >= 2 {
            last[1] |= 0x80;
        }
    }

    for data in frame_buf.drain(..) {
        let is_kf = is_h264_keyframe_packet(&data);
        if is_kf && !*in_keyframe {
            keyframe_buf.clear();
            *in_keyframe = true;
        }
        if *in_keyframe {
            keyframe_buf.push(data.clone());
            let marker = data.len() >= 2 && (data[1] & 0x80) != 0;
            if marker {
                shared.set_keyframe_cache(keyframe_buf.clone());
                log::info!("Cached keyframe: {} pkts, {} bytes",
                    keyframe_buf.len(),
                    keyframe_buf.iter().map(|p| p.len()).sum::<usize>());
                *in_keyframe = false;
            }
        }

        *rtp_count += 1;
        if *rtp_count <= 3 || *rtp_count % 500 == 0 {
            log::info!("broadcast_rtp #{} receivers={}", *rtp_count, shared.rtp_receiver_count());
        }
        shared.broadcast_rtp(data);
    }
}

fn apply_runtime_settings(
    rs: &Arc<runtime_settings::RuntimeSettings>,
    pipeline: &gstreamer::VideoPipeline,
) {
    if rs.take_keyframe_request() {
        pipeline.request_keyframe();
    }
    let new_bitrate = rs.video_bitrate_kbps();
    if new_bitrate != pipeline.config().bitrate {
        pipeline.set_bitrate(new_bitrate);
    }
    let new_ki = rs.keyframe_interval();
    if new_ki != pipeline.config().keyframe_interval {
        pipeline.set_keyframe_interval(new_ki);
    }
}

async fn run_async_services(
    config: Config,
    shared: Arc<web::SharedState>,
    runtime_settings: Arc<runtime_settings::RuntimeSettings>,
    _running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let upload_settings = file_upload::FileUploadSettings::from_config(&config);

    // Session manager (WebRTC)
    let session_manager = if config.webrtc.enabled {
        // Resolve a routable IP for the ICE-TCP candidate.
        // 0.0.0.0 is not valid in SDP — the browser needs an actual address.
        let bind_ip: std::net::IpAddr = config.http.host.parse().unwrap_or(
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        );
        let candidate_ip = if bind_ip.is_unspecified() {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        } else {
            bind_ip
        };
        let listen_addr = std::net::SocketAddr::new(candidate_ip, config.http.port);
        info!("ICE-TCP candidate address: {}", listen_addr);
        let sm = SessionManager::new(
            config.webrtc.clone(),
            shared.input_sender.clone(),
            upload_settings,
            runtime_settings.clone(),
            shared.clone(),
            16,
            listen_addr,
        );
        Some(Arc::new(sm))
    } else {
        None
    };

    // HTTP server
    let port = config.http.port;
    info!("Starting HTTP server on port {}", port);
    web::run_http_server_with_webrtc(port, shared.clone(), session_manager, config.http.tls)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("HTTP server error: {}", e).into()
        })?;

    Ok(())
}

fn apply_cli_overrides(config: &mut Config, args: &Args) {
    config.display.width = args.width;
    config.display.height = args.height;

    if let Some(port) = args.http_port {
        config.http.port = port;
    }
    if let Some(v) = args.basic_auth_enabled {
        config.http.basic_auth_enabled = v;
    }
    if let Some(ref u) = args.basic_auth_user {
        config.http.basic_auth_user = u.clone();
    }
    if let Some(ref p) = args.basic_auth_password {
        config.http.basic_auth_password = p.clone();
    }
    if let Some(v) = args.binary_clipboard_enabled {
        config.input.enable_binary_clipboard = v;
    }
    if let Some(v) = args.commands_enabled {
        config.input.enable_commands = v;
    }
    if let Some(ref ft) = args.file_transfers {
        config.input.file_transfers = ft.split(',').map(|s| s.trim().to_string()).collect();
    }
    if let Some(ref d) = args.upload_dir {
        config.input.upload_dir = d.clone();
    }
    if let Some(ref c) = args.webrtc_public_candidate {
        config.webrtc.public_candidate = Some(c.clone());
    }
    if let Some(v) = args.webrtc_candidate_from_host_header {
        config.webrtc.candidate_from_host_header = v;
    }
    if args.tls {
        config.http.tls = true;
    }
}

/// Set up GTK CSS to hide headerbars on fullscreen windows.
/// Dialogs are excluded so their controls stay visible.
fn setup_gtk_css_env() {
    // Only hide headerbar on fullscreen windows, exclude dialogs
    let css = "\
window.fullscreen:not(.dialog):not(.messagedialog) headerbar {\n\
  min-height: 0;\n\
  padding: 0;\n\
  margin: 0 0 -100px 0;\n\
  border: none;\n\
  background: none;\n\
  box-shadow: none;\n\
  opacity: 0;\n\
}\n\
window.fullscreen:not(.dialog):not(.messagedialog) headerbar * {\n\
  min-height: 0;\n\
  min-width: 0;\n\
  padding: 0;\n\
  margin: 0;\n\
}\n\
window.fullscreen:not(.dialog):not(.messagedialog) .titlebar {\n\
  min-height: 0;\n\
  padding: 0;\n\
  margin: 0 0 -100px 0;\n\
  border: none;\n\
  background: none;\n\
  box-shadow: none;\n\
  opacity: 0;\n\
}\n\
window.fullscreen:not(.dialog):not(.messagedialog) .titlebar * {\n\
  min-height: 0;\n\
  min-width: 0;\n\
  padding: 0;\n\
  margin: 0;\n\
}\n\
/* Ensure dialog action areas stay visible */\n\
.dialog actionbar,\n\
.messagedialog actionbar {\n\
  min-height: 40px;\n\
}\n";

    // Write CSS to ivnc-specific directory
    let runtime_dir = env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
    let ivnc_dir = format!("{}/ivnc", runtime_dir);
    if std::fs::create_dir_all(&ivnc_dir).is_err() {
        warn!("Failed to create ivnc runtime dir {}", ivnc_dir);
        return;
    }

    let css_path = format!("{}/gtk-headerbar.css", ivnc_dir);
    if std::fs::write(&css_path, css).is_err() {
        warn!("Failed to write GTK CSS to {}", css_path);
        return;
    }
    info!("Wrote GTK CSS to {}", css_path);

    // GTK_CSS tells GTK to load this CSS file in addition to the theme's CSS.
    env::set_var("GTK_CSS", &css_path);
    info!("Set GTK_CSS={}", css_path);
}
