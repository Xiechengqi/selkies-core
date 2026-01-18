# Deployment Guide

This document describes how to deploy selkies-core on a Linux host system.

## System Requirements

- Linux (X11-based desktop)
- Xvfb or X.org installed
- Rust 1.70+ toolchain
- libjpeg-turbo development files
- X11 development files
- **GStreamer 1.0+** (required for WebRTC mode)
- **Hardware encoder drivers** (optional, for hardware acceleration)

## Dependencies

### Ubuntu/Debian

#### 基础依赖

```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libjpeg-turbo8-dev \
    libx11-dev \
    libxcb1-dev \
    libxkbcommon-dev \
    xvfb \
    openbox
```

#### GStreamer 依赖（WebRTC 模式必需）

```bash
sudo apt-get install -y \
    gstreamer1.0-tools \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly \
    gstreamer1.0-x \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev
```

#### 硬件加速支持（可选）

```bash
# Intel VA-API
sudo apt-get install -y gstreamer1.0-vaapi libva-dev intel-media-va-driver-non-free

# NVIDIA NVENC（需要 NVIDIA 驱动）
sudo apt-get install -y gstreamer1.0-plugins-bad

# 音频支持（可选）
sudo apt-get install -y libpulse-dev libopus-dev libasound2-dev
```

### Fedora/RHEL

```bash
sudo dnf install -y \
    gcc \
    pkg-config \
    libjpeg-turbo-devel \
    libX11-devel \
    libxcb-devel \
    libxkbcommon-devel \
    pulseaudio-libs-devel \
    opus-devel \
    alsa-lib-devel \
    Xvfb \
    openbox
```

### Arch Linux

```bash
sudo pacman -S --needed \
    base-devel \
    libjpeg-turbo \
    libx11 \
    libxcb \
    xcb-util \
    libpulse \
    opus \
    alsa-lib \
    xorg-xvfb \
    openbox
```

## Building

### WebRTC 模式（默认）

```bash
cd /app/projects/selkies-core
cargo build --release
```

### WebSocket 备用模式（无需 GStreamer）

```bash
cargo build --release --no-default-features --features websocket-legacy
```

### 带硬件加速

```bash
# Intel VA-API
cargo build --release --features vaapi

# NVIDIA NVENC
cargo build --release --features nvenc

# Intel Quick Sync
cargo build --release --features qsv
```

### 带音频支持

```bash
cargo build --release --features audio
```

The binary will be at `target/release/selkies-core`.

## Installation

```bash
# Copy binary
sudo cp target/release/selkies-core /usr/local/bin/

# Create config directory
sudo mkdir -p /etc/selkies-core

# Copy default config
sudo cp config/selkies-core.toml /etc/selkies-core/

# Create runtime directory
sudo mkdir -p /var/run/selkies-core
sudo chown $USER:$USER /var/run/selkies-core
```

## Configuration

Edit `/etc/selkies-core/selkies-core.toml`:

### 基础配置

```toml
[display]
display = ":0"
width = 0                   # 0 = auto-detect
height = 0                  # 0 = auto-detect

[websocket]
host = "0.0.0.0"
port = 8080

[http]
port = 8000

[encoding]
target_fps = 30
jpeg_quality = 80
stripe_height = 16
```

### WebRTC 配置

```toml
[webrtc]
enabled = true
video_codec = "h264"        # h264, vp8, vp9, av1
video_bitrate = 4000        # kbps
hardware_encoder = "auto"   # auto, software, vaapi, nvenc, qsv
adaptive_bitrate = true
max_latency_ms = 100

[[webrtc.ice_servers]]
urls = ["stun:stun.l.google.com:19302"]
```

完整配置示例请参考 `config.example.toml`。

## Starting the Server

### Manual (Foreground)

```bash
selkies-core --foreground --verbose
```

### Systemd Service

Create `/etc/systemd/system/selkies-core.service`:

```ini
[Unit]
Description=Selkies Core Streaming Server
After=network.target

[Service]
Type=simple
User=selkies
Group=selkies
ExecStart=/usr/local/bin/selkies-core
Restart=always
RestartSec=5
Environment="DISPLAY=:0"

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable selkies-core
sudo systemctl start selkies-core
```

## Setting up X11 Desktop

### Using Xvfb

```bash
# Start Xvfb
Xvfb :0 -screen 0 1920x1080x24 &
export DISPLAY=:0

# Start Openbox
openbox &
```

### Using Real X Server

Ensure X server is running and `$DISPLAY` is set correctly.

## Running as Unprivileged User

For security, run selkies-core as a non-root user:

```bash
# Create dedicated user
sudo useradd -r selkies

# Set permissions
sudo chown -R selkies:selkies /var/run/selkies-core
```

Update systemd service to run as `selkies` user.

## Docker Deployment

### Dockerfile (WebRTC 模式)

```dockerfile
FROM rust:1.70 AS builder

RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libjpeg-turbo8-dev \
    libx11-dev \
    libxcb1-dev \
    libxkbcommon-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev

WORKDIR /app
COPY . .
RUN cargo build --release

FROM ubuntu:22.04

RUN apt-get update && apt-get install -y \
    xvfb \
    openbox \
    libjpeg-turbo8 \
    libx11-6 \
    libxcb1 \
    gstreamer1.0-tools \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-x \
    gstreamer1.0-vaapi \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/selkies-core /usr/local/bin/
COPY config.example.toml /etc/selkies-core.toml

EXPOSE 8000 8080

CMD ["selkies-core", "--config", "/etc/selkies-core.toml"]
```

## Verification

Check if the server is running:

```bash
# Check process
ps aux | grep selkies-core

# Check ports
ss -tlnp | grep 808

# Health check
curl http://localhost:8081/health
```

## Troubleshooting

### X11 Connection Failed

Ensure X server is running and `$DISPLAY` is set:

```bash
echo $DISPLAY
# Should show :0 or similar
```

### Permission Denied

Check X server permissions:

```bash
xhost +local:
```

### GStreamer 编码器未找到

检查可用的编码器：

```bash
gst-inspect-1.0 | grep -E "(x264|vp8|vaapi|nvenc|qsv)"
```

安装缺失的插件：

```bash
sudo apt-get install gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### 硬件编码器不可用

检查硬件支持：

```bash
# VA-API
vainfo

# NVIDIA
nvidia-smi
```

使用软件编码器作为备用：

```toml
[webrtc]
hardware_encoder = "software"
```

### WebRTC 连接失败

1. 检查防火墙设置（允许 UDP 端口）
2. 配置 TURN 服务器用于 NAT 穿透
3. 检查浏览器控制台错误信息

### Poor Performance

- Reduce target FPS
- Increase stripe height
- Lower video bitrate
- Use hardware acceleration

## Port Reference

| Port | Protocol | Description |
|------|----------|-------------|
| 8000 | HTTP | Web UI, health checks, WebRTC signaling |
| 8080 | WebSocket | WebSocket streaming (legacy mode) |
| UDP  | WebRTC | RTP media transport (dynamic ports) |
