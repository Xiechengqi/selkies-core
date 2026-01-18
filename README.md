# selkies-core

基于 Rust 的高性能 X11 桌面流媒体服务，支持 WebRTC + GStreamer 低延迟流媒体传输。

## 功能特性

- **WebRTC 流媒体** - 低延迟视频流，支持硬件加速编码
- **多编码器支持** - H.264, VP8, VP9, AV1
- **硬件加速** - Intel VA-API, NVIDIA NVENC, Intel Quick Sync Video
- **自适应比特率** - 基于网络状况动态调整比特率
- **输入注入** - 通过 WebRTC DataChannel 支持键盘/鼠标/剪贴板
- **双模式支持** - WebRTC（默认）+ WebSocket 备用模式
- **音频流媒体** - 可选音频捕获和流媒体传输
- **自动 X11 管理** - 无 DISPLAY 时自动创建虚拟 X11 服务器（Xvfb）
- **Web UI** - 内置 Web 界面，便于访问
- **HTTP API** - 健康检查和 Prometheus 指标端点

## 技术架构

### WebRTC 模式（默认）

```
X11 Screen → GStreamer ximagesrc → H.264/VP8 Encoder → RTP → WebRTC → Browser
Browser Input → RTCDataChannel → SCTP/DTLS → Parse → XTest → X11
```

### WebSocket 模式（备用）

```
X11 Screen → XShm Capture → RGB Frame → TurboJPEG → WebSocket → Browser
Browser Input → WebSocket Text → Parse → XTest → X11
```

### 模块结构

| 模块 | 功能 |
|------|------|
| `gstreamer/` | GStreamer 管道、编码器选择、屏幕捕获 |
| `webrtc/` | WebRTC 会话管理、信令、DataChannel |
| `display/` | 自动 X11 DISPLAY 检测和虚拟服务器管理 |
| `capture/` | X11 XShm 零拷贝屏幕捕获（备用模式） |
| `encode/` | 条纹化 JPEG 编码（备用模式） |
| `transport/` | WebSocket 服务器和 WebRTC 信令 |
| `input/` | XTest 鼠标/键盘事件注入 |
| `audio/` | 音频捕获和 Opus 编码（可选） |
| `web/` | Axum HTTP 服务器 |
| `config/` | TOML 配置管理 |

## 最新测试结果 (2026-01-18)

### ✅ WebRTC 模式验证通过

**测试环境:**
- OS: Ubuntu 24.04 (ARM64)
- GStreamer: 1.24.2
- Display: Xvnc :12 (1024x768)

**测试结果:**
- ✅ 编译成功（1分46秒，无错误）
- ✅ GStreamer 管道正常运行
- ✅ H.264 编码器工作正常（x264enc）
- ✅ RTP 包持续发送（2,000+ 包/分钟）
- ✅ HTTP API 端点全部响应正常
- ✅ WebRTC 信令端点就绪
- ✅ 客户端连接和输入事件处理正常

**性能指标:**
- CPU 使用: 242% (多核)
- 内存使用: 132 MB
- 帧率: 30 FPS
- 编码延迟: < 100ms

详细测试报告请参考 `WEBRTC_TODO.md`。

## 系统依赖

### 基础依赖

**Ubuntu/Debian:**

```bash
apt-get install build-essential pkg-config \
  libjpeg-turbo8-dev libx11-dev libxcb1-dev libxkbcommon-dev
```

### X11 虚拟显示（推荐）

用于无头环境或容器部署：

```bash
# Xvfb - X Virtual Framebuffer
apt-get install xvfb

# 验证安装
which Xvfb
```

**注意**: 如果启用了 `auto_x11` 配置（默认启用），selkies-core 会在没有可用 DISPLAY 时自动启动 Xvfb。

### GStreamer 依赖（WebRTC 模式必需）

```bash
# 核心 GStreamer 包
apt-get install \
  gstreamer1.0-tools \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-x \
  libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev
```

### 硬件加速支持（可选）

```bash
# Intel VA-API
apt-get install gstreamer1.0-vaapi libva-dev

# NVIDIA NVENC（需要 NVIDIA 驱动）
apt-get install gstreamer1.0-plugins-bad

# Intel Quick Sync Video
apt-get install intel-media-va-driver-non-free
```

### 音频支持（可选）

```bash
apt-get install libpulse-dev libopus-dev libasound2-dev
```

## 编译

### WebRTC 模式（默认）

```bash
cargo build --release
```

### WebSocket 备用模式（无 GStreamer）

```bash
cargo build --release --no-default-features --features websocket-legacy
```

### 带硬件加速支持

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

编译输出：`target/release/selkies-core`

## 运行

### 自动 X11 管理（推荐）

selkies-core 会自动检测和管理 X11 DISPLAY：

```bash
# 自动模式 - 无 DISPLAY 时自动创建 Xvfb
./target/release/selkies-core

# 输出示例：
# INFO  selkies_core > Display manager initialized successfully
# INFO  selkies_core > Created managed display: :99 (Xvfb)
# INFO  selkies_core > Using display: :99
```

### 使用现有 DISPLAY

```bash
# 使用环境变量
DISPLAY=:0 ./target/release/selkies-core

# 或通过配置文件指定
./target/release/selkies-core --config config.toml
```

### 禁用自动 X11 管理

```bash
# 禁用自动创建 Xvfb
./target/release/selkies-core --no-auto-x11
```

### 自定义 X11 配置

```bash
# 自定义 DISPLAY 编号范围
./target/release/selkies-core --x11-display-range "100-150"

# 强制使用 Xvfb 后端
./target/release/selkies-core --x11-backend xvfb

# 自定义启动超时（秒）
./target/release/selkies-core --x11-startup-timeout 20
```

### 使用配置文件

```bash
./target/release/selkies-core --config config.toml
```

### 覆盖端口

```bash
./target/release/selkies-core --port 8080 --http-port 8000
```

### 前台调试模式

```bash
SELKIES_LOG=debug ./target/release/selkies-core --verbose
```

### 配置文件示例

复制示例配置文件：

```bash
cp config.example.toml config.toml
```

编辑 `config.toml` 以自定义设置。

## Web 界面

内置的 Selkies 前端通过 HTTP 端口提供（默认 `8000`）。

在浏览器中打开：

```
http://localhost:8000/
```

### WebRTC 连接

WebRTC 信令通过 `/webrtc` WebSocket 端点提供：

```
ws://localhost:8000/webrtc
```

### WebSocket 备用连接

WebSocket 服务器默认监听 `8080` 端口：

```
ws://localhost:8080/websockets
```

## HTTP 端点

| 端点 | 说明 |
|------|------|
| `GET /` | Web 界面 |
| `GET /health` | 健康检查（JSON） |
| `GET /metrics` | Prometheus 指标 |
| `GET /clients` | 活跃连接列表 |
| `GET /ui-config` | UI 配置 |
| `GET /ws-config` | WebSocket 配置 |
| `GET /webrtc` | WebRTC 信令 WebSocket（升级） |

## 配置选项

配置文件采用 TOML 格式，主要配置项：

### 基础配置

```toml
[display]
display = ":0"              # X11 显示
width = 0                   # 屏幕宽度（0 = 自动检测）
height = 0                  # 屏幕高度（0 = 自动检测）

# 自动 X11 管理配置
auto_x11 = true             # 启用自动 X11 管理（默认：true）
x11_backend = "auto"        # X11 后端：auto, xvfb, xdummy, none
x11_display_range = [99, 199]  # 自动分配的 DISPLAY 编号范围
x11_startup_timeout = 10    # Xvfb 启动超时（秒）
x11_extra_args = []         # 传递给 Xvfb 的额外参数

[websocket]
host = "0.0.0.0"            # WebSocket 监听地址
port = 8080                 # WebSocket 端口

[http]
port = 8000                 # HTTP 端口

[encoding]
target_fps = 30             # 目标帧率
jpeg_quality = 80           # JPEG 质量 (1-100)
stripe_height = 16          # 条纹高度
```

### WebRTC 配置

```toml
[webrtc]
enabled = true              # 启用 WebRTC
video_codec = "h264"        # 视频编码器：h264, vp8, vp9, av1
video_bitrate = 4000        # 目标比特率（kbps）
video_bitrate_max = 8000    # 最大比特率（kbps）
video_bitrate_min = 500     # 最小比特率（kbps）
hardware_encoder = "auto"   # 硬件编码器：auto, software, vaapi, nvenc, qsv
adaptive_bitrate = true     # 自适应比特率
max_latency_ms = 100        # 最大延迟（毫秒）
keyframe_interval = 60      # 关键帧间隔（帧数）

# ICE 服务器配置
[[webrtc.ice_servers]]
urls = ["stun:stun.l.google.com:19302"]
```

完整配置示例请参考 `config.example.toml`。

## 自动 X11 管理

selkies-core 内置了智能 X11 DISPLAY 管理功能，可在无头环境或容器中自动创建虚拟 X11 服务器。

### 工作原理

1. **自动检测** - 启动时检查 `DISPLAY` 环境变量和配置文件
2. **智能创建** - 如果没有可用 DISPLAY，自动启动 Xvfb
3. **编号分配** - 自动分配可用的 DISPLAY 编号（默认 :99-:199）
4. **生命周期管理** - 随 selkies-core 进程启动/终止自动管理 Xvfb

### 使用场景

**场景 1：Docker 容器（无 DISPLAY）**

```bash
# 容器内自动创建 Xvfb
docker run -p 8000:8000 selkies-core
# 输出: Created managed display: :99 (Xvfb)
```

**场景 2：有现有 DISPLAY**

```bash
# 自动使用现有 DISPLAY
DISPLAY=:0 ./selkies-core
# 输出: Using existing display: :0
```

**场景 3：多实例部署**

```bash
# 自动分配不同的 DISPLAY 编号
./selkies-core --x11-display-range "100-200"  # 实例 1 使用 :100
./selkies-core --x11-display-range "200-300"  # 实例 2 使用 :200
```

### 配置选项详解

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `auto_x11` | `true` | 启用自动 X11 管理 |
| `x11_backend` | `"auto"` | 后端选择：auto（自动检测）、xvfb、xdummy、none |
| `x11_display_range` | `[99, 199]` | DISPLAY 编号范围，避免与常见的 :0-:10 冲突 |
| `x11_startup_timeout` | `10` | Xvfb 启动超时（秒） |
| `x11_extra_args` | `[]` | 传递给 Xvfb 的额外参数 |

### 命令行参数

```bash
# 禁用自动 X11 管理
--no-auto-x11

# 强制使用特定后端
--x11-backend xvfb

# 自定义 DISPLAY 范围
--x11-display-range "100-150"

# 自定义启动超时
--x11-startup-timeout 20
```

### 降级策略

selkies-core 使用多层降级策略确保最大兼容性：

```
1. 检查 DISPLAY 环境变量
   ↓ 不可用
2. 检查配置文件中的 display
   ↓ 不可用
3. 自动创建 Xvfb（如果 auto_x11=true）
   ↓ 失败
4. 返回错误，提示用户手动配置
```

### Xvfb 启动参数

自动创建的 Xvfb 使用以下参数：

```bash
Xvfb :99 \
  -screen 0 1920x1080x24 \
  -dpi 96 \
  -nolisten tcp \
  -noreset \
  +extension GLX \
  +extension RANDR \
  +extension RENDER
```

### 高级用法

**自定义 Xvfb 参数**

```toml
[display]
x11_extra_args = ["-fbdir", "/var/tmp", "-ac"]
```

**禁用特定环境**

```bash
# 生产环境禁用自动创建
export SELKIES_NO_AUTO_X11=1
./selkies-core --no-auto-x11
```

**调试模式**

```bash
# 查看详细的 X11 管理日志
SELKIES_LOG=debug ./selkies-core --verbose
```

## 协议

### WebRTC 信令协议

WebRTC 信令使用 JSON 格式通过 WebSocket 传输：

```json
// Offer
{"type": "offer", "sdp": "...", "session_id": "..."}

// Answer
{"type": "answer", "sdp": "...", "session_id": "..."}

// ICE Candidate
{"type": "ice_candidate", "candidate": "...", "sdp_mid": "0", "sdp_mline_index": 0}
```

### WebRTC DataChannel 输入协议

输入事件通过 DataChannel 传输，格式与 WebSocket 模式兼容：

| 格式 | 说明 |
|------|------|
| `m,{x},{y}` | 鼠标移动 |
| `b,{button},{pressed}` | 鼠标按键 |
| `w,{dx},{dy}` | 鼠标滚轮 |
| `k,{keysym},{pressed}` | 键盘事件 |
| `t,{text}` | 文本输入 |
| `c,{base64}` | 剪贴板数据 |

### WebSocket 备用协议

| 方向 | 格式 | 说明 |
|------|------|------|
| S→C | `s,{y},{h},{base64}` | 视频条纹 |
| S→C | `a,{base64}` | 音频数据 |
| C→S | `m,{x},{y}` | 鼠标移动 |
| C→S | `k,{keysym},{pressed}` | 键盘事件 |

## Docker 部署

### Dockerfile 示例

```dockerfile
FROM rust:1.70 AS builder

# 安装依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libjpeg-turbo8-dev \
    libx11-dev \
    libxcb1-dev \
    libxkbcommon-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev

WORKDIR /build
COPY . .
RUN cargo build --release

FROM ubuntu:22.04

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    libjpeg-turbo8 \
    libx11-6 \
    libxcb1 \
    xvfb \
    gstreamer1.0-tools \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-x \
    gstreamer1.0-vaapi \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/selkies-core /usr/local/bin/
COPY config.example.toml /etc/selkies-core.toml

EXPOSE 8000 8080

CMD ["selkies-core", "--config", "/etc/selkies-core.toml"]
```

## 性能特点

### WebRTC 模式

- **硬件加速编码** - VA-API, NVENC, QSV 支持
- **低延迟流媒体** - 典型延迟 < 100ms
- **自适应比特率** - 根据网络状况动态调整
- **RTP 传输** - 高效的实时传输协议
- **GStreamer 管道** - 成熟的多媒体框架

### WebSocket 模式

- **XShm 零拷贝** - 高效屏幕捕获
- **条纹化编码** - 仅传输变化区域
- **xxHash64** - 快速变化检测
- **TurboJPEG** - 高性能 JPEG 编码

### 通用优化

- **mimalloc** - 高性能内存分配
- **LTO 优化** - 链接时优化
- **异步 I/O** - Tokio 异步运行时

## 硬件编码器设置

### Intel VA-API

```bash
# 安装驱动
apt-get install intel-media-va-driver-non-free

# 验证
vainfo

# 配置
[webrtc]
hardware_encoder = "vaapi"
```

### NVIDIA NVENC

```bash
# 确保 NVIDIA 驱动已安装
nvidia-smi

# 配置
[webrtc]
hardware_encoder = "nvenc"
```

### Intel Quick Sync

```bash
# 安装驱动
apt-get install intel-media-va-driver-non-free

# 配置
[webrtc]
hardware_encoder = "qsv"
```

## 故障排除

### 自动 X11 管理问题

**Xvfb 未安装**

```bash
# 错误信息：No X11 backend available. Please install Xvfb
apt-get install xvfb

# 验证安装
which Xvfb
```

**DISPLAY 范围耗尽**

```bash
# 错误信息：No available display number in the configured range
# 解决方案：扩大 DISPLAY 范围或清理未使用的 X11 进程

# 查看占用的 DISPLAY
ls /tmp/.X11-unix/

# 清理僵尸 X11 进程
pkill -9 Xvfb

# 或在配置中扩大范围
[display]
x11_display_range = [99, 299]
```

**权限问题**

```bash
# 错误信息：Permission denied
# 确保有权限创建 /tmp/.X11-unix/ 下的 socket 文件

# 检查权限
ls -la /tmp/.X11-unix/

# 如需要，以 root 运行或调整权限
chmod 1777 /tmp/.X11-unix/
```

**禁用自动管理**

```bash
# 如果不需要自动 X11 管理
./target/release/selkies-core --no-auto-x11

# 或在配置文件中禁用
[display]
auto_x11 = false
```

### GStreamer 编码器未找到

```bash
# 列出可用的编码器
gst-inspect-1.0 | grep -E "(x264|vp8|vaapi|nvenc|qsv)"

# 安装缺失的插件
apt-get install gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
```

### 硬件编码器不可用

```bash
# 检查 VA-API 支持
vainfo

# 检查 NVIDIA 支持
nvidia-smi

# 使用软件编码器作为备用
[webrtc]
hardware_encoder = "software"
```

### WebRTC 连接失败

1. 检查防火墙设置（UDP 端口）
2. 配置 TURN 服务器用于 NAT 穿透
3. 检查浏览器控制台错误信息

### 高延迟或卡顿

```toml
# 降低比特率
[webrtc]
video_bitrate = 2000

# 增加关键帧频率
keyframe_interval = 30

# 降低分辨率
[display]
width = 1280
height = 720
```

## 架构迁移说明

selkies-core 已从 **WebSocket + TurboJPEG** 架构迁移到 **WebRTC + GStreamer** 架构。

### 主要变化

- **默认模式**: WebRTC 流媒体（低延迟，硬件加速）
- **备用模式**: WebSocket 流媒体（兼容性，无需 GStreamer）
- **新增模块**: `gstreamer/` 和 `webrtc/` 模块
- **协议兼容**: DataChannel 输入协议与 WebSocket 模式兼容

### 迁移指南

如果您需要使用旧的 WebSocket 模式：

```bash
# 编译时禁用 WebRTC
cargo build --release --no-default-features --features websocket-legacy

# 或在配置文件中禁用
[webrtc]
enabled = false
```

## 限制说明

- 仅支持 X11（无 Wayland 支持）
- WebRTC 模式需要 GStreamer 1.0+
- 硬件编码器需要相应的驱动支持

## 许可证

详见 LICENSE 文件。
