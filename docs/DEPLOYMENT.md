# Deployment Guide

本文档描述如何在 Linux 主机上部署 iVnc。

## 系统要求

- Linux（内置 Smithay Wayland 合成器，无需外部 X11/Wayland）
- Rust 1.75+ 工具链
- GStreamer 1.0+
- PulseAudio 或 PipeWire（音频捕获）
- 硬件编码器驱动（可选）

## 依赖安装

### Ubuntu/Debian

#### 编译依赖

```bash
sudo apt-get update
sudo apt-get install -y \
    build-essential \
    pkg-config \
    cmake \
    curl \
    ca-certificates \
    libx11-dev \
    libxcb1-dev \
    libxkbcommon-dev \
    libssl-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev \
    libpulse-dev \
    libopus-dev \
    libwayland-dev \
    libpixman-1-dev \
    libinput-dev \
    libudev-dev \
    libseat-dev
```

#### GStreamer 运行时

```bash
sudo apt-get install -y \
    gstreamer1.0-tools \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly \
    gstreamer1.0-x
```

#### 音频支持

```bash
# PipeWire（推荐）
sudo apt-get install -y pipewire pipewire-pulse pipewire-media-session

# 或 PulseAudio
sudo apt-get install -y pulseaudio
```

#### 硬件加速（可选）

```bash
# Intel VA-API
sudo apt-get install -y gstreamer1.0-vaapi libva-dev intel-media-va-driver-non-free

# NVIDIA NVENC（需要 NVIDIA 驱动）
sudo apt-get install -y gstreamer1.0-plugins-bad
```

### Fedora/RHEL

```bash
sudo dnf install -y \
    gcc cmake \
    pkg-config \
    libX11-devel \
    libxcb-devel \
    libxkbcommon-devel \
    openssl-devel \
    gstreamer1-devel \
    gstreamer1-plugins-base-devel \
    pulseaudio-libs-devel \
    opus-devel \
    wayland-devel \
    pixman-devel \
    libinput-devel \
    systemd-devel \
    libseat-devel \
    pipewire pipewire-pulseaudio
```

### Arch Linux

```bash
sudo pacman -S --needed \
    base-devel cmake \
    libx11 libxcb xcb-util \
    libxkbcommon openssl \
    gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad \
    libpulse opus \
    wayland pixman libinput \
    pipewire pipewire-pulse
```

## Smithay 依赖

iVnc 依赖本地 smithay 仓库（需放在项目同级目录）：

```bash
git clone https://github.com/Smithay/smithay.git ../smithay
cd ../smithay && git checkout 3d3f9e359352d95cffd1e53287d57df427fcbd34
```

## 编译

```bash
cd /path/to/iVnc

# 使用 build.sh（推荐）
bash build.sh --release

# 或直接使用 cargo
cargo build --release
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

构建产物位于 `target/release/ivnc`，`build.sh` 会自动复制到项目根目录。

## 安装

```bash
# 复制二进制
sudo cp target/release/ivnc /usr/local/bin/

# 复制配置文件
sudo cp config.example.toml /etc/ivnc.toml

# 按需编辑配置
sudo vi /etc/ivnc.toml
```

## 配置

编辑 `/etc/ivnc.toml`：

### 基础配置

```toml
[display]
width = 1920
height = 1080
refresh_rate = 60

[http]
host = "0.0.0.0"
port = 8008
basic_auth_enabled = true
basic_auth_user = "user"
basic_auth_password = "change_me"

[encoding]
target_fps = 30
max_fps = 60
```

### WebRTC 配置

```toml
[webrtc]
enabled = true
tcp_only = true
video_codec = "h264"
video_bitrate = 8000
video_bitrate_max = 16000
video_bitrate_min = 1000
hardware_encoder = "auto"
keyframe_interval = 60
candidate_from_host_header = true
# 公网部署时设置外部地址
# public_candidate = "1.2.3.4:8008"
```

### 音频配置

```toml
[audio]
enabled = true
sample_rate = 48000
channels = 2
bitrate = 128000
```

完整配置示例请参考 `config.example.toml`。

## 启动服务

### 手动启动（前台）

```bash
# 确保 PulseAudio/PipeWire 可用
export XDG_RUNTIME_DIR=/run/user/$(id -u)

# 启动 PipeWire（如未自动启动）
pipewire &
pipewire-media-session &
pipewire-pulse &

# 启动 iVnc
ivnc -c /etc/ivnc.toml --verbose
```

### Systemd Service

创建 `/etc/systemd/system/ivnc.service`：

```ini
[Unit]
Description=iVnc Wayland Desktop Streaming Server
After=network.target pipewire.service

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/ivnc -c /etc/ivnc.toml
Restart=always
RestartSec=5
Environment="XDG_RUNTIME_DIR=/run/user/0"

[Install]
WantedBy=multi-user.target
```

启用并启动：

```bash
sudo systemctl daemon-reload
sudo systemctl enable ivnc
sudo systemctl start ivnc
```

## Docker 部署

### Dockerfile

```dockerfile
FROM rust:1.75 AS builder

RUN apt-get update && apt-get install -y \
    pkg-config cmake libx11-dev libxcb1-dev libxkbcommon-dev libssl-dev \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    libpulse-dev libopus-dev libwayland-dev libpixman-1-dev \
    libinput-dev libudev-dev libseat-dev

WORKDIR /build
COPY . .
RUN cargo build --release

FROM ubuntu:22.04

RUN apt-get update && apt-get install -y \
    libx11-6 libxcb1 libpulse0 \
    pipewire pipewire-pulse \
    gstreamer1.0-tools gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
    gstreamer1.0-plugins-ugly gstreamer1.0-x \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ivnc /usr/local/bin/
COPY config.example.toml /etc/ivnc.toml

EXPOSE 8008

ENV XDG_RUNTIME_DIR=/run/user/0

CMD ["ivnc", "-c", "/etc/ivnc.toml"]
```

## 验证

```bash
# 检查进程
ps aux | grep ivnc

# 检查端口
ss -tlnp | grep 8008

# 健康检查
curl http://localhost:8008/health

# 检查音频
pactl info
```

浏览器访问 `http://<host>:8008/` 即可使用。

## 故障排除

### WebRTC 连接失败

1. 确认浏览器能访问 HTTP 端口
2. 检查浏览器控制台是否有 ICE/DTLS 错误
3. 如果通过反向代理，确保 WebSocket 和 TCP 连接能正确转发到同一端口
4. 公网部署时需设置 `public_candidate` 或启用 `candidate_from_host_header`

### 无音频

1. 确认 PulseAudio/PipeWire 正在运行：`pactl info`
2. 确认 `XDG_RUNTIME_DIR` 环境变量已设置
3. 确认配置文件中 `[audio] enabled = true`
4. 检查日志中是否有 "PulseAudio capture opened" 消息
5. 可通过 `PULSE_SOURCE` 环境变量指定音频源

### GStreamer 编码器未找到

```bash
gst-inspect-1.0 | grep -E "(x264|openh264|vp8|vaapi|nvenc|qsv)"
sudo apt-get install gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### 硬件编码器不可用

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

### 高延迟或卡顿

- 降低分辨率和比特率
- 减小 keyframe_interval
- 使用硬件加速编码

```toml
[webrtc]
video_bitrate = 4000
keyframe_interval = 30

[display]
width = 1280
height = 720
```

## 端口说明

| 端口 | 协议 | 说明 |
|------|------|------|
| 8008 | HTTP/WS/TCP | Web UI、健康检查、WebRTC 信令、ICE-TCP（同端口复用） |
