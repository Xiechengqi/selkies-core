# iVnc 详细设计文档

## 1. 项目概述

### 1.1 项目背景

iVnc 是一个用 Rust 编写的高性能 Web 桌面流媒体服务。它内置 Smithay Wayland 合成器，使用 str0m Sans-I/O WebRTC 库 + GStreamer 实现低延迟视频/音频流媒体传输，通过浏览器即可访问远程桌面。

### 1.2 设计目标

| 目标 | 描述 |
|------|------|
| **单一二进制** | 编译为单一可执行文件，内置 Wayland 合成器和 Web 前端资源 |
| **高性能** | GStreamer 硬件加速编码 + str0m Sans-I/O WebRTC，低延迟传输 |
| **同端口复用** | HTTP、WebSocket 信令、ICE-TCP 共享同一端口，简化部署 |
| **低资源占用** | mimalloc 分配器，按需渲染，无变化时不编码 |
| **完整桌面体验** | 键盘/鼠标/剪贴板/文件传输/音频/任务栏 |

### 1.3 技术栈

| 层级 | 技术选型 |
|------|----------|
| 语言 | Rust 2021 Edition |
| 异步运行时 | Tokio（异步服务）+ calloop（合成器事件循环） |
| Wayland 合成器 | Smithay（headless backend） |
| 视频编码 | GStreamer（H.264/VP8/VP9/AV1，支持 VA-API/NVENC/QSV） |
| WebRTC | str0m Sans-I/O（ICE-lite，RTP 模式，TCP 传输） |
| HTTP 服务 | Axum |
| 音频捕获 | PulseAudio（cpal + libpulse） |
| 音频编码 | Opus |
| 内存分配 | mimalloc |

---

## 2. 系统架构

### 2.1 整体架构图

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Browser (iVnc Frontend)                        │
│                                                                             │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────────┐ │
│  │  Video (WebRTC)  │  │  Audio (WebRTC)  │  │  DataChannel (Input/Ctrl)  │ │
│  └────────┬────────┘  └────────┬────────┘  └──────────────┬──────────────┘ │
│           │                    │                          │                 │
└───────────┼────────────────────┼──────────────────────────┼─────────────────┘
            │ SRTP/TCP           │ SRTP/TCP                 │ SCTP/DTLS/TCP
            │ (RFC 4571)         │ (RFC 4571)               │ (RFC 4571)
            ▼                    ▼                          ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           iVnc (Rust Binary)                                │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐ │
│  │                    Axum HTTP Server (port 8008)                        │ │
│  │         HTTP / WebSocket 信令 / ICE-TCP 同端口复用                     │ │
│  │                   (首字节分类自动区分)                                  │ │
│  └───────────┬─────────────────┬─────────────────────────┬───────────────┘ │
│              │                 │                         │                  │
│              ▼                 ▼                         ▼                  │
│  ┌───────────────────┐ ┌───────────────────┐ ┌───────────────────────────┐ │
│  │  str0m RtcSession │ │  GStreamer Pipeline│ │  Smithay Compositor       │ │
│  │  (Sans-I/O)       │ │                   │ │  (Headless Backend)       │ │
│  │                   │ │ ┌───────────────┐ │ │                           │ │
│  │ - ICE-lite        │ │ │ appsrc        │ │ │ - XDG Shell              │ │
│  │ - DTLS/SRTP       │ │ │ → videoconvert│ │ │ - 窗口管理               │ │
│  │ - RTP mode        │ │ │ → encoder     │ │ │ - 输入注入               │ │
│  │ - DataChannel     │ │ │ → rtph264pay  │ │ │ - 剪贴板同步             │ │
│  │ - NullPacer       │ │ │ → appsink     │ │ │ - 任务栏                 │ │
│  │                   │ │ └───────────────┘ │ │                           │ │
│  └───────────────────┘ └───────────────────┘ └───────────────────────────┘ │
│                                                                             │
│  ┌───────────────────┐ ┌───────────────────┐                               │
│  │  Audio Capture    │ │  File Upload      │                               │
│  │  (PulseAudio)     │ │  Handler          │                               │
│  │  → Opus Encoder   │ │                   │                               │
│  └───────────────────┘ └───────────────────┘                               │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 数据流图

```
视频流:
═══════

Smithay Compositor (headless framebuffer)
      │
      ▼ render_frame() → RGBA 像素
┌─────────────┐
│ GStreamer    │ appsrc → videoconvert → encoder → rtph264pay → appsink
│ Pipeline    │
└──────┬──────┘
       │
       ▼ try_pull_sample() → RTP 包
┌─────────────┐
│ Main Loop   │ 设置 marker bit，缓存 keyframe
│ RTP 广播    │
└──────┬──────┘
       │
       ▼ broadcast_rtp() → tokio broadcast channel
┌─────────────┐
│ RtcSession  │ write_video_rtp() → str0m write_rtp()
│ Drive Loop  │ → SRTP 加密 → RFC 4571 TCP 帧 → TCP write
└──────┬──────┘
       │
       ▼
   Browser (WebRTC RTCPeerConnection)


输入流:
═══════

Browser (RTCDataChannel)
      │
      ▼ SCTP/DTLS → str0m Event::ChannelData
┌─────────────┐
│ RtcSession  │ handle_datachannel_data() → 解析文本协议
│ Drive Loop  │
└──────┬──────┘
       │
       ▼ input_tx (mpsc channel)
┌─────────────┐
│ Main Loop   │ drain_input_events()
│ 输入注入    │
└──────┬──────┘
       │
       ▼ Smithay seat API
┌─────────────┐
│ Wayland     │ pointer.motion / keyboard.input / etc.
│ Clients     │
└─────────────┘


剪贴板流 (浏览器 → 远程应用):
═════════════════════════════

Browser clipboard → DataChannel "cw,{base64}" → clipboard_incoming_rx
      → set_data_device_selection() → Wayland client reads via send_selection()
      → clipboard_suppress_until 防止回声循环

剪贴板流 (远程应用 → 浏览器):
═════════════════════════════

Wayland client copies → new_selection() → clipboard_pending_mime (延迟)
      → main loop: request_data_device_client_selection() + flush_clients()
      → 非阻塞 pipe 读取 → base64 编码 → text_sender broadcast
      → DataChannel → Browser clipboard
```

### 2.3 线程模型

```
┌─────────────────────────────────────────────────────────────────┐
│                     Main Thread (calloop)                        │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │              Compositor Event Loop                       │   │
│  │                                                         │   │
│  │  - Wayland 协议处理 (event_loop.dispatch)               │   │
│  │  - 延迟剪贴板请求 (clipboard_pending_mime)              │   │
│  │  - 浏览器剪贴板注入 (clipboard_incoming_rx)             │   │
│  │  - 输入事件注入 (drain_input_events)                    │   │
│  │  - 非阻塞剪贴板 pipe 读取                               │   │
│  │  - 光标/任务栏状态广播                                   │   │
│  │  - 帧渲染 + GStreamer push                              │   │
│  │  - RTP 拉取 + 广播 (pull_and_broadcast_rtp)             │   │
│  │  - 统计信息更新                                         │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                     Tokio Runtime                                │
│                                                                 │
│  ┌─────────────────┐  ┌─────────────────┐                      │
│  │ Axum HTTP Server │  │ Signaling WS    │                      │
│  │ (port 8008)      │  │ (/webrtc)       │                      │
│  └─────────────────┘  └─────────────────┘                      │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │ RtcSession Drive Tasks (每个 WebRTC 连接一个)            │   │
│  │                                                         │   │
│  │  tokio::select! {                                       │   │
│  │    TCP read → RFC 4571 decode → str0m handle_input      │   │
│  │    RTP broadcast → write_video_rtp → drain_outputs      │   │
│  │    Audio broadcast → write_audio_rtp → drain_outputs    │   │
│  │    Text broadcast → DataChannel write                   │   │
│  │    Timeout → str0m handle_input(Timeout)                │   │
│  │    Ping interval → keepalive                            │   │
│  │  }                                                      │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                   Audio Capture Thread                           │
│                                                                 │
│  PulseAudio → Opus 编码 → audio_sender broadcast               │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 2.4 通信通道

| 通道 | 类型 | 方向 | 用途 |
|------|------|------|------|
| `rtp_sender` | tokio broadcast | Main → RtcSession | 视频 RTP 包广播 |
| `audio_sender` | tokio broadcast | Audio Thread → RtcSession | 音频 Opus 包广播 |
| `text_sender` | tokio broadcast | Main → RtcSession | 光标/剪贴板/统计/任务栏消息 |
| `input_tx` | mpsc unbounded | RtcSession → Main | 输入事件（键盘/鼠标/窗口操作） |
| `clipboard_incoming_rx` | mpsc | RtcSession → Main | 浏览器剪贴板数据 |

---

## 3. 模块设计

### 3.1 模块结构

```
iVnc/
├── Cargo.toml
├── build.sh                   # 构建脚本
├── config.example.toml        # 配置示例
├── src/
│   ├── main.rs                # 入口点、合成器主循环、RTP 广播
│   ├── lib.rs                 # 库入口
│   ├── args.rs                # CLI 参数解析 (clap)
│   │
│   ├── config/                # 配置管理
│   │   ├── mod.rs             # TOML 配置加载、验证
│   │   └── ui.rs              # UI 配置（环境变量驱动）
│   │
│   ├── compositor/            # Smithay Wayland 合成器
│   │   ├── mod.rs             # 模块导出
│   │   ├── state.rs           # Compositor 状态结构体
│   │   ├── headless.rs        # Headless backend（渲染、输出管理）
│   │   ├── handlers/
│   │   │   ├── mod.rs         # SeatHandler、SelectionHandler、DataDevice
│   │   │   ├── compositor.rs  # CompositorHandler（surface commit）
│   │   │   └── xdg_shell.rs   # XDG Shell（窗口创建/配置/销毁）
│   │   └── grabs/
│   │       ├── mod.rs
│   │       ├── move_grab.rs   # 窗口移动
│   │       └── resize_grab.rs # 窗口缩放
│   │
│   ├── gstreamer/             # GStreamer 视频管道
│   │   ├── mod.rs             # PipelineConfig、VideoPipeline
│   │   ├── pipeline.rs        # appsrc → encoder → rtppay → appsink
│   │   └── encoder.rs         # 编码器选择（auto/software/vaapi/nvenc/qsv）
│   │
│   ├── webrtc/                # str0m WebRTC 层
│   │   ├── mod.rs             # SessionManager、WebRTCError
│   │   ├── rtc_session.rs     # RtcSession + drive_session 事件循环
│   │   ├── session.rs         # 会话管理、ICE-TCP 连接匹配
│   │   ├── tcp_framing.rs     # RFC 4571 TCP 帧编解码
│   │   ├── media_track.rs     # RTP 工具函数
│   │   ├── data_channel.rs    # DataChannel 输入解析
│   │   └── signaling.rs       # WebRTC 信令协议
│   │
│   ├── transport/             # 信令传输
│   │   ├── mod.rs
│   │   └── signaling_server.rs # WebSocket 信令服务器
│   │
│   ├── web/                   # HTTP 服务
│   │   ├── mod.rs
│   │   ├── http_server.rs     # Axum 路由、同端口复用
│   │   ├── shared.rs          # SharedState（跨线程共享状态）
│   │   └── embedded_assets.rs # 嵌入式前端资源
│   │
│   ├── audio/                 # 音频模块
│   │   ├── mod.rs             # AudioConfig、run_audio_capture
│   │   └── runtime.rs         # PulseAudio 捕获 + Opus 编码
│   │
│   ├── input.rs               # InputEvent 枚举、InputEventData 结构
│   ├── clipboard.rs           # ClipboardReceiver（DataChannel 剪贴板处理）
│   ├── system_clipboard.rs    # 系统剪贴板工具
│   ├── file_upload.rs         # 文件上传处理
│   └── runtime_settings.rs    # 运行时设置（比特率/帧率动态调整）
│
├── web/ivnc/                  # 前端资源
│   ├── index.html
│   ├── ivnc-wr-core.js        # 主客户端逻辑
│   └── lib/
│       └── webrtc.js          # WebRTC 连接管理
│
└── docs/
    ├── DESIGN.md              # 本文档
    ├── PROTOCOL.md            # 协议规范
    └── DEPLOYMENT.md          # 部署指南
```

### 3.2 核心模块职责

#### 3.2.1 compositor 模块

Smithay headless Wayland 合成器，管理窗口生命周期和输入注入。

关键组件：
- `Compositor` 状态结构体：包含 Space、Seat、窗口注册表、剪贴板状态
- `HeadlessBackend`：无 GPU 的软件渲染后端，输出 RGBA 像素
- `SelectionHandler`：Wayland 剪贴板协议，延迟读取模式避免 smithay 时序问题
- `XdgShellHandler`：窗口创建时自动全屏（非对话框），窗口销毁时清理注册表

#### 3.2.2 gstreamer 模块

GStreamer 视频编码管道，将 RGBA 帧编码为 RTP 包。

管道结构：
```
appsrc (RGBA) → videoconvert → capsfilter → encoder → rtph264pay → appsink
```

支持的编码器：
- 软件：x264enc, vp8enc, vp9enc, av1enc
- VA-API：vaapih264enc, vaapivp8enc
- NVENC：nvh264enc
- QSV：qsvh264enc

#### 3.2.3 webrtc 模块

基于 str0m Sans-I/O 的 WebRTC 实现。

`RtcSession`：
- ICE-lite 模式，仅提供 TCP passive candidate
- RTP 模式，GStreamer RTP 包通过 `write_rtp()` 传入
- NullPacer，每次 `handle_timeout()` → `poll_output()` 发射一个包
- DataChannel 用于输入事件和控制消息

`drive_session()`：
- 每个 WebRTC 连接一个 tokio task
- `tokio::select!` 多路复用 TCP I/O、RTP/Audio/Text 广播、超时
- 每个 RTP 包写入后立即 `handle_timeout()` + `drain_outputs_fast()` 确保及时发送

`SessionManager`：
- 管理 ICE-TCP 连接匹配（通过 ufrag 关联信令和 TCP 连接）
- 会话生命周期管理

#### 3.2.4 web 模块

Axum HTTP 服务器，同端口复用 HTTP/WebSocket/ICE-TCP。

`SharedState`：
- 跨线程共享状态（Arc 包装）
- broadcast channel 用于 RTP/Audio/Text 广播
- 统计信息、显示尺寸、keyframe 缓存
- `datachannel_open_count` 用于触发任务栏重发

同端口复用原理：
- TCP 连接到达时，读取首字节判断协议类型
- HTTP 请求（`GET`/`POST`）→ Axum 路由
- ICE-TCP 数据包 → 转交 SessionManager 匹配会话

---

## 4. 关键机制

### 4.1 剪贴板同步

#### 浏览器 → 远程应用

1. 浏览器通过 DataChannel 发送 `cw,{base64}` 消息
2. `clipboard_incoming_rx` 接收，base64 解码为文本
3. 调用 `set_data_device_selection()` 设置合成器剪贴板
4. 设置 `clipboard_suppress_until`（500ms 窗口）防止回声循环
5. Wayland 客户端通过 `send_selection()` 读取剪贴板内容

回声循环防护：当合成器设置剪贴板后，焦点 Wayland 客户端（如 Chromium）会重新断言自己的 `wl_data_source`，触发 `new_selection` 回调。`clipboard_suppress_until` 在 500ms 内抑制这些重新断言，避免旧内容覆盖新内容。

#### 远程应用 → 浏览器

1. Wayland 客户端复制内容，触发 `new_selection()` 回调
2. `new_selection()` 仅保存 mime type 到 `clipboard_pending_mime`（延迟模式）
3. 主循环 `event_loop.dispatch()` 后，smithay 已更新 seat selection
4. 创建非阻塞 pipe，调用 `request_data_device_client_selection()`
5. 立即 `flush_clients()` 确保客户端收到 fd
6. 后续循环迭代中非阻塞读取 pipe 数据
7. EOF 时 base64 编码，通过 `text_sender` 广播到所有 DataChannel

延迟读取原因：smithay 在 `new_selection()` 返回后才更新 `seat_data.clipboard_selection`，在回调内调用 `request_data_device_client_selection()` 会失败。

### 4.2 任务栏同步

1. `window_registry` 维护窗口列表（稳定顺序）
2. 窗口创建/销毁时设置 `taskbar_dirty = true`
3. 主循环检测 dirty 标志，构建 JSON 窗口列表（id, title, app_id, display_name, focused）
4. 通过 `text_sender` 广播 `taskbar,{json}` 消息
5. 新 DataChannel 打开时（`datachannel_open_count` 变化），清空缓存强制重发

### 4.3 RTP 广播与 Keyframe 缓存

1. GStreamer appsink 产出 RTP 包
2. 主循环 `pull_and_broadcast_rtp()` 拉取所有可用包
3. 按 RTP timestamp 分组为帧，最后一个包设置 marker bit
4. 检测 H.264 keyframe NAL（IDR/SPS/PPS），缓存完整 keyframe
5. 通过 `broadcast_rtp()` 发送到 tokio broadcast channel
6. 新连接建立后可立即发送缓存的 keyframe，加速首帧显示

### 4.4 同端口复用

```
TCP 连接到达 (port 8008)
      │
      ▼ 读取首字节
      │
      ├── 'G'/'P'/'H'/'D'/'O'/'C' → HTTP 请求 → Axum 路由
      │                                          ├── GET / → Web UI
      │                                          ├── GET /webrtc → WebSocket 升级 → 信令
      │                                          ├── GET /health → 健康检查
      │                                          └── ...
      │
      └── 其他字节 → ICE-TCP 数据包 → SessionManager
                                       → 匹配 ufrag → RtcSession drive loop
```

### 4.5 帧率控制与按需渲染

- 主循环以 `target_fps` 为目标帧率运行
- 仅当 `needs_redraw = true` 时才渲染和编码（surface commit 触发）
- 无活跃会话时跳过渲染
- 每秒至少渲染一次（有会话时），确保浏览器有可解码帧
- `send_frame_callbacks()` 在 sleep 前调用，给客户端完整帧周期准备下一帧

---

## 5. 配置

### 5.1 配置文件格式 (TOML)

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
hardware_encoder = "auto"
keyframe_interval = 60
pipeline_latency_ms = 50
candidate_from_host_header = true
# public_candidate = "1.2.3.4:8008"
```

### 5.2 环境变量

| 变量名 | 描述 |
|--------|------|
| `XDG_RUNTIME_DIR` | PulseAudio/PipeWire socket 目录（音频必需） |
| `PULSE_SOURCE` | 指定 PulseAudio 音频源 |
| `IVNC_ENCODER` | 编码器选项 |
| `IVNC_FRAMERATE` | 帧率或帧率范围 |
| `IVNC_AUDIO_ENABLED` | 启用音频 |
| `IVNC_UI_*` | UI 配置（值后加 `\|locked` 可锁定） |

---

## 6. 参考资料

- [Smithay](https://github.com/Smithay/smithay) - Wayland 合成器库
- [str0m](https://github.com/algesten/str0m) - Sans-I/O WebRTC 库
- [GStreamer](https://gstreamer.freedesktop.org/) - 多媒体框架
- [RFC 4571](https://tools.ietf.org/html/rfc4571) - RTP over TCP framing

---

*文档版本: 2.0*
*最后更新: 2026-02-16*
