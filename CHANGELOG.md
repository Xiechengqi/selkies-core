# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- WebRTC streaming support with GStreamer integration
- Hardware-accelerated encoding (VA-API, NVENC, QSV)
- Adaptive bitrate control
- Multiple video codec support (H.264, VP8, VP9, AV1)
- WebRTC signaling server
- DataChannel for input events
- Session management with automatic cleanup
- Comprehensive configuration options for WebRTC
- Build scripts (build.sh, test.sh)
- Docker and Docker Compose support
- Makefile for simplified builds
- CI/CD configuration (GitHub Actions)
- Detailed documentation (DEPLOYMENT.md, PROTOCOL.md)
- Migration summary document

### Changed
- Architecture migrated from WebSocket+TurboJPEG to WebRTC+GStreamer
- HTTP server now includes WebRTC signaling endpoint
- Configuration format extended with WebRTC options
- README updated with WebRTC documentation

### Fixed
- Stripe structure now includes height field
- Compilation warnings cleaned up
- Import issues in http_server.rs resolved

## [0.2.0] - 2026-01-18

### Added
- Initial WebRTC + GStreamer implementation
- Dual-mode support (WebRTC + WebSocket)

## [0.1.0] - Previous

### Added
- WebSocket streaming with TurboJPEG encoding
- X11 screen capture with XShm
- Input injection (keyboard, mouse, clipboard)
- Audio streaming support
- HTTP health check endpoints
