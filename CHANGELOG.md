# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Bidirectional clipboard sync between browser and remote Wayland applications
  - Browser → remote: `set_data_device_selection()` with 500ms echo suppression
  - Remote → browser: deferred `request_data_device_client_selection()` with non-blocking pipe read
- Taskbar window list broadcast via DataChannel (`taskbar,{json}`)
  - Window focus/close control from browser (`focus,{id}`, `close,{id}`)
  - `display_name` resolution from `.desktop` files
  - Automatic resend on new DataChannel open (`datachannel_open_count`)
- Cursor state broadcast (`cursor,{"override":"..."}`)
- Text input injection via `zwp_text_input_v3` protocol
- Keyboard reset command (`kr`) to release stuck modifier keys
- Dynamic display resize from browser (`r,{width}x{height}`)
- GTK CSS injection to hide headerbars on fullscreen windows (preserving dialog controls)
- `gtk3-nocsd` LD_PRELOAD for GTK3 CSD suppression
- Dialog window detection (skip fullscreen, preserve headerbar)

### Changed
- WebRTC migrated from webrtc-rs to str0m Sans-I/O library
  - ICE-lite mode with TCP passive candidates
  - RTP mode with NullPacer (no BWE)
  - Per-packet interleaved `write_rtp()` + `drain_outputs_fast()` for timely delivery
  - RFC 4571 TCP framing for all WebRTC traffic
- HTTP/WebSocket signaling/ICE-TCP share single port (default 8008)
  - First-byte classification to distinguish HTTP from ICE-TCP
- Clipboard `new_selection()` uses deferred read pattern
  - smithay updates seat selection AFTER `new_selection()` returns
  - Mime type saved to `clipboard_pending_mime`, actual read happens in main loop
  - `flush_clients()` after `request_data_device_client_selection()` for immediate fd delivery
- Taskbar resend trigger changed from `receiver_count()` to `datachannel_open_count`
  - `receiver_count()` increases at `subscribe_text()` time (before DC is open)
  - `datachannel_open_count` bumps on actual `Event::ChannelOpen`
- Architecture documentation completely rewritten for current implementation
- Protocol documentation updated for WebRTC DataChannel protocol

### Fixed
- Remote app clipboard not syncing to browser (deferred read pattern)
- Browser clipboard requiring two copies to update remote app (echo suppression)
- Taskbar not showing after WebRTC migration (datachannel_open_count fix)
- Dialog windows losing headerbar controls when fullscreened
- Keyboard focus not working until first pointer enter (Chromium Ozone/Wayland)

## [0.3.0] - 2026-02-16

### Added
- str0m Sans-I/O WebRTC implementation
- Single-port multiplexing (HTTP + WebSocket + ICE-TCP)
- Keyframe caching for fast first-frame display
- Per-session tokio task with `tokio::select!` event loop
- Ping/pong keepalive (15s interval, 45s timeout)
- Client statistics forwarding (`_f`, `_l`, `_stats_video`, `_stats_audio`)
- Runtime settings adjustment via DataChannel (`SETTINGS,{json}`)

### Changed
- Replaced webrtc-rs with str0m for WebRTC
- Simplified ICE to TCP-only passive candidates

## [0.2.0] - 2026-01-18

### Added
- Initial WebRTC + GStreamer implementation
- Hardware-accelerated encoding (VA-API, NVENC, QSV)
- Multiple video codec support (H.264, VP8, VP9, AV1)
- WebRTC signaling server
- DataChannel for input events
- Session management with automatic cleanup
- Comprehensive configuration options for WebRTC
- Build scripts (build.sh)
- Detailed documentation (DEPLOYMENT.md, PROTOCOL.md)

### Changed
- Architecture migrated from WebSocket+TurboJPEG to WebRTC+GStreamer
- HTTP server now includes WebRTC signaling endpoint
- Configuration format extended with WebRTC options

## [0.1.0] - Previous

### Added
- WebSocket streaming with TurboJPEG encoding
- X11 screen capture with XShm
- Input injection (keyboard, mouse, clipboard)
- Audio streaming support
- HTTP health check endpoints
