# iVnc

基于 Rust 的高性能 Wayland 桌面流媒体服务，内置 Smithay 合成器，使用 str0m Sans-I/O WebRTC 库 + GStreamer 实现低延迟流媒体传输。

## 功能特性

- **Wayland 合成器** - 内置 Smithay headless 合成器，无需外部 X11/Wayland 服务
- **str0m Sans-I/O WebRTC** - 基于 str0m 的纯 Rust WebRTC 实现，ICE-lite 模式，TCP 传输
- **同端口复用** - HTTP、WebSocket 信令、ICE-TCP 共享同一端口
- **多编码器支持** - H.264, VP8, VP9, AV1
- **硬件加速** - Intel VA-API, NVIDIA NVENC, Intel Quick Sync Video
- **输入转发** - 通过 WebRTC DataChannel 支持键盘/鼠标/文本输入（IME）
- **双向剪贴板** - 浏览器 ↔ 远程应用剪贴板同步，500ms 回声抑制
- **任务栏** - 窗口列表广播，支持从浏览器切换焦点/关闭窗口
- **光标同步** - 远程光标样式实时同步到浏览器
- **音频流媒体** - PulseAudio/PipeWire 捕获 + Opus 编码
- **文件传输** - 支持上传/下载文件
- **Web UI** - 内置 Web 界面，便于访问
- **HTTP API** - 健康检查和 Prometheus 指标端点
- **Basic Auth** - 内置 HTTP 基础认证

## 技术架构

### 流媒体管道

```
视频流 (Server → Browser):
┌──────────────┐    ┌─────────────────────────────────────────────┐    ┌──────────────┐
│   Smithay    │    │              GStreamer Pipeline              │    │    str0m      │
│  Compositor  │───▶│ appsrc → videoconvert → encoder → rtppay    │───▶│  write_rtp()  │
│  (headless)  │    │                         H.264/VP8/VP9/AV1   │    │  Sans-I/O     │
└──────────────┘    └─────────────────────────────────────────────┘    └──────┬───────┘
   RGBA 帧                                                                    │
                                                                    SRTP 加密 │ NullPacer
                                                                              ▼
                    ┌─────────────┐    ┌──────────────────┐    ┌──────────────────┐
                    │   Browser   │◀───│  RFC 4571 TCP    │◀───│  poll_output()   │
                    │  (WebRTC)   │    │  帧封装 (同端口)  │    │  drain_outputs() │
                    └─────────────┘    └──────────────────┘    └──────────────────┘

音频流 (Server → Browser):
┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐
│  PulseAudio  │───▶│    Opus      │───▶│    str0m     │───▶│  SRTP → TCP 帧   │───▶ Browser
│  /PipeWire   │    │   Encoder    │    │  write_rtp() │    │  (RFC 4571)      │
└──────────────┘    └──────────────┘    └──────────────┘    └──────────────────┘

输入流 (Browser → Server):
┌──────────────┐    ┌──────────────────┐    ┌──────────────┐    ┌──────────────┐
│   Browser    │───▶│  RTCDataChannel  │───▶│    str0m     │───▶│   Smithay    │
│  键盘/鼠标   │    │  SCTP/DTLS/TCP   │    │  ChannelData │    │  Seat 注入   │
└──────────────┘    └──────────────────┘    └──────────────┘    └──────────────┘
```

### WebRTC 传输层

iVnc 使用 str0m Sans-I/O WebRTC 库，所有 I/O 由调用方驱动：

- **ICE-lite 模式** - 服务端仅提供 TCP passive candidate，不主动探测
- **RTP 模式** - GStreamer 产出的 RTP 包通过 `write_rtp()` 直接传入 str0m，str0m 负责 SRTP 加密、SSRC 分配、RTP header extension 注入
- **NullPacer** - BWE 默认关闭，使用 NullPacer，每次 `handle_timeout()` → `poll_output()` 循环发射一个包
- **同端口复用** - 通过首字节分类区分 HTTP 请求和 ICE-TCP 数据包

### 双向剪贴板同步

浏览器 → 远程应用：
- DataChannel `cw,{base64}` → `set_data_device_selection()` → Wayland 客户端读取
- 500ms `clipboard_suppress_until` 窗口防止回声循环（Wayland 客户端重新断言 `wl_data_source`）

远程应用 → 浏览器：
- Wayland 客户端复制 → `new_selection()` 保存 mime type（延迟模式）
- 主循环 `event_loop.dispatch()` 后调用 `request_data_device_client_selection()` + `flush_clients()`
- 非阻塞 pipe 读取 → base64 编码 → DataChannel `clipboard,{base64}` 广播

延迟读取原因：smithay 在 `new_selection()` 返回后才更新 `seat_data.clipboard_selection`，回调内直接读取会失败。

### 任务栏窗口管理

- `window_registry` 维护窗口列表（稳定顺序）
- 窗口创建/销毁时通过 DataChannel 广播 `taskbar,{json}`（包含 id, title, app_id, display_name, focused）
- 浏览器可发送 `focus,{id}` / `close,{id}` 控制窗口
- `display_name` 从 `.desktop` 文件解析
- 新 DataChannel 打开时（`datachannel_open_count` 变化）自动重发窗口列表

### 模块结构

| 模块 | 功能 |
|------|------|
| `compositor/` | Smithay Wayland 合成器（headless backend） |
| `gstreamer/` | GStreamer 管道、编码器选择、RTP 打包 |
| `webrtc/rtc_session.rs` | str0m Sans-I/O 会话驱动（事件循环、RTP 转发、DataChannel） |
| `webrtc/session.rs` | 会话管理、ICE-TCP 连接匹配 |
| `webrtc/tcp_framing.rs` | RFC 4571 TCP 帧编解码 |
| `transport/` | WebRTC 信令服务器（WebSocket） |
| `input.rs` | 键盘/鼠标事件处理 |
| `audio/` | PulseAudio/PipeWire 捕获和 Opus 编码 |
| `web/` | Axum HTTP 服务器、同端口复用、嵌入式前端资源 |
| `config/` | TOML 配置管理、UI 配置 |
| `clipboard.rs` | 剪贴板同步 |
| `file_upload.rs` | 文件上传处理 |

## 系统依赖

### 编译依赖

```bash
apt-get install build-essential pkg-config curl ca-certificates cmake \
  libx11-dev libxcb1-dev libxkbcommon-dev libssl-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  libpulse-dev libopus-dev \
  libwayland-dev libpixman-1-dev libinput-dev libudev-dev libseat-dev
```

### Smithay 与 str0m 依赖

smithay 和 str0m 通过 Git URL 引用，cargo 构建时自动拉取，无需手动 clone。

### GStreamer 运行时

```bash
apt-get install \
  gstreamer1.0-tools \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly \
  gstreamer1.0-x
```

### 音频运行时

音频捕获需要 PulseAudio 或 PipeWire（带 PulseAudio 兼容层）：

```bash
# PipeWire（推荐）
apt-get install pipewire pipewire-pulse pipewire-media-session

# 或 PulseAudio
apt-get install pulseaudio
```

启动 PipeWire（如未通过 systemd 自动启动）：

```bash
export XDG_RUNTIME_DIR=/run/user/$(id -u)
pipewire &
pipewire-media-session &
pipewire-pulse &
```

iVnc 会自动检测默认 sink 的 monitor source 来捕获桌面音频输出。也可通过 `PULSE_SOURCE` 环境变量指定音频源。

### 硬件加速（可选）

```bash
# Intel VA-API
apt-get install gstreamer1.0-vaapi libva-dev

# NVIDIA NVENC（需要 NVIDIA 驱动）
apt-get install gstreamer1.0-plugins-bad

# Intel Quick Sync Video
apt-get install intel-media-va-driver-non-free
```

## 编译

使用 `build.sh` 脚本（推荐）：

```bash
# Release 构建（默认包含 PulseAudio 音频支持）
bash build.sh --release

# Debug 构建
bash build.sh --debug
```

构建完成后二进制文件位于项目根目录：`./ivnc`

也可以直接使用 cargo：

```bash
cargo build --release
# 输出：target/release/ivnc
```

### Cargo Features

| Feature | 说明 | 默认 |
|---------|------|------|
| `pulseaudio` | PulseAudio 音频捕获 + Opus 编码 | ✅ |
| `audio` | cpal 音频捕获 + Opus 编码 | |
| `vaapi` | Intel VA-API 硬件编码 | |
| `nvenc` | NVIDIA NVENC 硬件编码 | |
| `qsv` | Intel Quick Sync Video | |

## 运行

```bash
# 使用默认配置（/etc/ivnc.toml，不存在则使用内置默认值）
./ivnc

# 指定配置文件
./ivnc -c config.toml

# 覆盖端口和分辨率
./ivnc -c config.toml --http-port 8008 --width 1920 --height 1080

# 调试模式
./ivnc -c config.toml --verbose
```

音频捕获需要 `XDG_RUNTIME_DIR` 环境变量指向 PulseAudio socket 所在目录：

```bash
XDG_RUNTIME_DIR=/run/user/$(id -u) ./ivnc -c config.toml --http-port 8008
```

### 命令行参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `-c, --config` | `/etc/ivnc.toml` | 配置文件路径 |
| `--width` | `1920` | 显示宽度 |
| `--height` | `1080` | 显示高度 |
| `--http-port` | 配置文件值 | HTTP 端口（同时用于 ICE-TCP） |
| `--basic-auth-enabled` | `true` | 启用基础认证 |
| `--basic-auth-user` | | 认证用户名 |
| `--basic-auth-password` | | 认证密码 |
| `-v, --verbose` | | 详细日志 |
| `--foreground` | | 前台运行 |

完整参数列表：`./ivnc --help`

### 配置文件

复制示例配置：

```bash
cp config.example.toml config.toml
```

主要配置段：

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
basic_auth_password = "mypasswd"

[encoding]
target_fps = 30
max_fps = 60

[audio]
enabled = true
sample_rate = 48000
channels = 2
bitrate = 128000

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
# public_candidate = "1.2.3.4:8008"
```

完整配置示例见 `config.example.toml`。

### 环境变量

| 环境变量 | 说明 |
|----------|------|
| `XDG_RUNTIME_DIR` | PulseAudio/PipeWire socket 目录（音频捕获必需） |
| `PULSE_SOURCE` | 指定 PulseAudio 音频源（默认自动检测 monitor source） |
| `IVNC_ENCODER` | 编码器选项（逗号分隔） |
| `IVNC_FRAMERATE` | 帧率或帧率范围（如 `30` 或 `15-60`） |
| `IVNC_AUDIO_ENABLED` | 启用音频 (`true`/`false`) |
| `IVNC_AUDIO_BITRATE` | 音频比特率或范围 |
| `IVNC_MOUSE_ENABLED` | 启用鼠标 |
| `IVNC_KEYBOARD_ENABLED` | 启用键盘 |
| `IVNC_CLIPBOARD_ENABLED` | 启用剪贴板 |
| `IVNC_MANUAL_WIDTH` | 手动分辨率宽度 |
| `IVNC_MANUAL_HEIGHT` | 手动分辨率高度 |
| `IVNC_UI_SHOW_SIDEBAR` | 显示侧边栏 |

UI 相关环境变量值后加 `|locked` 可锁定前端不可修改。

## Web 界面

内置前端通过 HTTP 端口提供：

```
http://localhost:8008/
```

WebRTC 信令通过 WebSocket（同端口）：

```
ws://localhost:8008/webrtc
```

ICE-TCP 连接也复用同一端口，通过首字节分类自动区分。

## HTTP 端点

| 端点 | 说明 |
|------|------|
| `GET /` | Web 界面 |
| `GET /health` | 健康检查（JSON） |
| `GET /metrics` | Prometheus 指标 |
| `GET /clients` | 活跃连接列表 |
| `GET /ui-config` | UI 配置 |
| `GET /ws-config` | WebSocket 端口配置 |
| `GET /webrtc` | WebRTC 信令 WebSocket |

## DataChannel 协议

输入事件和控制消息通过 WebRTC DataChannel 传输。

### 客户端 → 服务端

| 格式 | 说明 |
|------|------|
| `m,{x},{y},{buttonMask},{0}` | 鼠标移动（buttonMask 变化时合成按键事件） |
| `b,{button},{pressed}` | 鼠标按键 |
| `w,{dx},{dy}` | 鼠标滚轮 |
| `k,{keysym},{pressed}` | 键盘事件（X11 KeySym） |
| `t,{text}` | IME 文本输入（zwp_text_input_v3） |
| `cw,{base64}` | 剪贴板内容 |
| `r,{width}x{height}` | 分辨率调整 |
| `focus,{id}` | 切换窗口焦点 |
| `close,{id}` | 关闭窗口 |
| `kr` | 键盘重置（释放所有修饰键） |
| `pong` | 心跳响应 |

### 服务端 → 客户端

| 格式 | 说明 |
|------|------|
| `cursor,{json}` | 光标样式变化 |
| `clipboard,{base64}` | 剪贴板内容 |
| `taskbar,{json}` | 窗口列表更新 |
| `stats,{json}` | 性能统计（每秒） |
| `ping` | 心跳请求 |

完整协议规范见 [docs/PROTOCOL.md](docs/PROTOCOL.md)。

## Docker 部署

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

CMD ["ivnc", "--config", "/etc/ivnc.toml"]
```

## 故障排除

### GStreamer 编码器未找到

```bash
gst-inspect-1.0 | grep -E "(x264|openh264|vp8|vaapi|nvenc|qsv)"
apt-get install gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### WebRTC 连接失败

1. 确认浏览器能访问 HTTP 端口
2. 检查浏览器控制台是否有 ICE/DTLS 错误
3. 如果通过反向代理，确保 WebSocket 和 TCP 连接能正确转发

### 无音频

1. 确认 PulseAudio/PipeWire 正在运行：`pactl info`
2. 确认 `XDG_RUNTIME_DIR` 环境变量已设置
3. 确认配置文件中 `[audio] enabled = true`
4. 检查日志中是否有 "PulseAudio capture opened" 消息
5. 浏览器自动播放策略要求用户交互（点击/按键）后才能播放音频

### 高延迟或卡顿

```toml
[webrtc]
video_bitrate = 4000
keyframe_interval = 30

[display]
width = 1280
height = 720
```

## 许可证

详见 LICENSE 文件。
