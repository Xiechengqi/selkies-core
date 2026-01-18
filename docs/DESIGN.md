# Selkies-Core 详细设计文档

## 1. 项目概述

### 1.1 项目背景

Selkies-Core 是一个用 Rust 重写的高性能 Web 桌面流媒体服务，用于替代原有的 Python 实现。它提供通过 WebSocket 将 Linux 桌面实时传输到浏览器的能力。

### 1.2 设计目标

| 目标 | 描述 |
|------|------|
| **单一二进制** | 编译为单一静态链接可执行文件，便于部署 |
| **高性能** | 利用 Rust 的零成本抽象，实现低延迟、低 CPU 占用 |
| **兼容性** | 完全兼容现有 Selkies 前端，无需修改 |
| **简化部署** | 无需 Python、GStreamer 等复杂依赖 |
| **低资源占用** | 目标内存占用 < 50MB，CPU 占用 < 10% (静态场景) |

### 1.3 非目标

- 不支持 WebRTC 模式（仅支持 WebSocket 模式）
- 不支持 GPU 硬件加速编码（仅 CPU 编码）
- 不支持游戏手柄
- 不支持 Wayland（仅 X11）

### 1.4 技术栈

| 层级 | 技术选型 |
|------|----------|
| 语言 | Rust 2021 Edition |
| 异步运行时 | Tokio |
| WebSocket | tokio-tungstenite |
| HTTP 服务 | Axum |
| 屏幕捕获 | xcap (XShm) |
| 图像编码 | turbojpeg (libjpeg-turbo) |
| 变化检测 | xxhash |
| 输入模拟 | x11rb (XTest) |
| 音频捕获 | cpal + libpulse |
| 音频编码 | opus-rs |

---

## 2. 系统架构

### 2.1 整体架构图

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Browser (Selkies Frontend)                      │
│                                                                             │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────────┐ │
│  │  Video Canvas   │  │  Audio Context  │  │  Input Event Listeners      │ │
│  └────────┬────────┘  └────────┬────────┘  └──────────────┬──────────────┘ │
│           │                    │                          │                 │
└───────────┼────────────────────┼──────────────────────────┼─────────────────┘
            │ WebSocket          │ WebSocket                │ WebSocket
            │ (JPEG stripes)     │ (Opus audio)             │ (Input events)
            ▼                    ▼                          ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           selkies-core (Rust Binary)                         │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐ │
│  │                         WebSocket Server                               │ │
│  │                      (tokio-tungstenite)                               │ │
│  └───────────┬─────────────────┬─────────────────────────┬───────────────┘ │
│              │                 │                         │                  │
│              ▼                 ▼                         ▼                  │
│  ┌───────────────────┐ ┌───────────────────┐ ┌───────────────────────────┐ │
│  │   Video Pipeline  │ │   Audio Pipeline  │ │     Input Handler         │ │
│  │                   │ │                   │ │                           │ │
│  │ ┌───────────────┐ │ │ ┌───────────────┐ │ │ ┌───────────────────────┐ │ │
│  │ │ Screen Capture│ │ │ │ Audio Capture │ │ │ │ Protocol Parser       │ │ │
│  │ │ (xcap/XShm)   │ │ │ │ (PulseAudio)  │ │ │ └───────────┬───────────┘ │ │
│  │ └───────┬───────┘ │ │ └───────┬───────┘ │ │             │             │ │
│  │         │         │ │         │         │ │             ▼             │ │
│  │         ▼         │ │         ▼         │ │ ┌───────────────────────┐ │ │
│  │ ┌───────────────┐ │ │ ┌───────────────┐ │ │ │ X11 Input Simulator   │ │ │
│  │ │ Stripe Encoder│ │ │ │ Opus Encoder  │ │ │ │ (XTest Extension)     │ │ │
│  │ │ (turbojpeg)   │ │ │ │ (opus-rs)     │ │ │ └───────────────────────┘ │ │
│  │ └───────────────┘ │ │ └───────────────┘ │ │                           │ │
│  └───────────────────┘ └───────────────────┘ └───────────────────────────┘ │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
            │                    │                          │
            ▼                    ▼                          ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Linux System                                    │
│                                                                             │
│  ┌───────────────────┐ ┌───────────────────┐ ┌───────────────────────────┐ │
│  │   X11 Server      │ │   PulseAudio      │ │   X11 (XTest)             │ │
│  │   (Xvfb/Xorg)     │ │   Daemon          │ │   Input Injection         │ │
│  └───────────────────┘ └───────────────────┘ └───────────────────────────┘ │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 数据流图

```
Screen Capture Flow:
═══════════════════

X11 Framebuffer
      │
      ▼ XShm (零拷贝)
┌─────────────┐
│ Raw Frame   │ BGRA, 1920x1080, ~8MB
│ (xcap)      │
└──────┬──────┘
       │
       ▼ 分割为条纹 (64px height)
┌─────────────┐
│ Stripes     │ 17 stripes @ 1920x64
│ (64px each) │
└──────┬──────┘
       │
       ▼ XXHash 变化检测
┌─────────────┐
│ Changed     │ 只处理变化的条纹
│ Stripes     │
└──────┬──────┘
       │
       ▼ JPEG 编码 (turbojpeg)
┌─────────────┐
│ JPEG Data   │ ~5-50KB per stripe
└──────┬──────┘
       │
       ▼ Base64 + 协议封装
┌─────────────┐
│ WebSocket   │ "s,{y},{h},{base64}"
│ Message     │
└──────┬──────┘
       │
       ▼
   Browser


Input Event Flow:
═════════════════

Browser Input
      │
      ▼
┌─────────────┐
│ WebSocket   │ "m,1024,768" / "k,65,1"
│ Message     │
└──────┬──────┘
       │
       ▼ 协议解析
┌─────────────┐
│ Input Event │ MouseMove{x,y} / KeyPress{keysym}
│ Struct      │
└──────┬──────┘
       │
       ▼ XTest 注入
┌─────────────┐
│ X11 Server  │ 模拟输入事件
└─────────────┘
```

### 2.3 线程模型

```
┌─────────────────────────────────────────────────────────────────┐
│                        Tokio Runtime                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────────┐                                            │
│  │  Main Task      │  程序入口，初始化和协调                      │
│  └────────┬────────┘                                            │
│           │                                                     │
│           ├──────────────────┬──────────────────┐               │
│           │                  │                  │               │
│           ▼                  ▼                  ▼               │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ Capture Task    │ │ Audio Task      │ │ WebSocket Task  │   │
│  │                 │ │                 │ │                 │   │
│  │ - 屏幕捕获      │ │ - 音频捕获      │ │ - 连接管理      │   │
│  │ - 条纹编码      │ │ - Opus 编码     │ │ - 消息路由      │   │
│  │ - 变化检测      │ │                 │ │ - 输入处理      │   │
│  │                 │ │                 │ │                 │   │
│  │ [CPU-bound]     │ │ [CPU-bound]     │ │ [IO-bound]      │   │
│  └────────┬────────┘ └────────┬────────┘ └────────┬────────┘   │
│           │                  │                  │               │
│           └──────────────────┴──────────────────┘               │
│                              │                                  │
│                              ▼                                  │
│                    ┌─────────────────┐                          │
│                    │ Broadcast Channel│                          │
│                    │ (多客户端广播)   │                          │
│                    └─────────────────┘                          │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. 模块设计

### 3.1 模块结构

```
selkies-core/
├── Cargo.toml
├── Cargo.lock
├── build.rs                    # 构建脚本 (链接 C 库)
├── src/
│   ├── main.rs                 # 入口点
│   ├── lib.rs                  # 库入口
│   ├── config.rs               # 配置管理
│   ├── error.rs                # 错误类型定义
│   │
│   ├── capture/                # 屏幕捕获模块
│   │   ├── mod.rs
│   │   ├── x11.rs              # X11 XShm 捕获
│   │   └── frame.rs            # 帧数据结构
│   │
│   ├── encode/                 # 编码模块
│   │   ├── mod.rs
│   │   ├── stripe.rs           # 条纹分割
│   │   ├── jpeg.rs             # JPEG 编码
│   │   └── change_detect.rs    # 变化检测
│   │
│   ├── audio/                  # 音频模块
│   │   ├── mod.rs
│   │   ├── capture.rs          # PulseAudio 捕获
│   │   └── opus.rs             # Opus 编码
│   │
│   ├── transport/              # 传输层
│   │   ├── mod.rs
│   │   ├── server.rs           # WebSocket 服务器
│   │   ├── client.rs           # 客户端连接管理
│   │   └── protocol.rs         # Selkies 协议
│   │
│   ├── input/                  # 输入处理模块
│   │   ├── mod.rs
│   │   ├── parser.rs           # 输入消息解析
│   │   ├── x11.rs              # X11 XTest 输入
│   │   └── keysym.rs           # 键码映射表
│   │
│   ├── web/                    # HTTP 服务模块
│   │   ├── mod.rs
│   │   └── static_files.rs     # 静态文件服务
│   │
│   └── utils/                  # 工具模块
│       ├── mod.rs
│       └── metrics.rs          # 性能指标
│
├── static/                     # 前端静态资源
│   └── (从 Selkies 复制)
│
└── docs/
    ├── DESIGN.md               # 本文档
    ├── PROTOCOL.md             # 协议规范
    └── DEPLOYMENT.md           # 部署指南
```

### 3.2 模块职责

#### 3.2.1 capture 模块

**职责**: X11 屏幕捕获

**公开接口**:
```rust
pub struct X11Capturer {
    // ...
}

impl X11Capturer {
    /// 创建新的捕获器
    pub fn new(display: &str) -> Result<Self>;

    /// 捕获单帧
    pub fn capture_frame(&mut self) -> Result<Frame>;

    /// 获取屏幕尺寸
    pub fn get_dimensions(&self) -> (u32, u32);
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,      // BGRA 格式
    pub timestamp: Instant,
}
```

#### 3.2.2 encode 模块

**职责**: 视频帧编码

**公开接口**:
```rust
pub struct StripeEncoder {
    // ...
}

impl StripeEncoder {
    /// 创建编码器
    pub fn new(config: EncoderConfig) -> Result<Self>;

    /// 编码帧为条纹
    pub fn encode(&mut self, frame: &Frame) -> Result<Vec<EncodedStripe>>;

    /// 强制全帧刷新
    pub fn force_refresh(&mut self);
}

pub struct EncoderConfig {
    pub quality: u8,           // JPEG 质量 1-100
    pub stripe_height: u32,    // 条纹高度 (像素)
}

pub struct EncodedStripe {
    pub y: u32,                // Y 坐标
    pub height: u32,           // 条纹高度
    pub data: Vec<u8>,         // JPEG 数据
    pub is_keyframe: bool,     // 是否为关键帧
}
```

#### 3.2.3 audio 模块

**职责**: 音频捕获和编码

**公开接口**:
```rust
pub struct AudioCapture {
    // ...
}

impl AudioCapture {
    /// 创建音频捕获器
    pub fn new(config: AudioConfig) -> Result<Self>;

    /// 开始捕获
    pub async fn start(&mut self, tx: mpsc::Sender<AudioPacket>) -> Result<()>;

    /// 停止捕获
    pub fn stop(&mut self);
}

pub struct AudioConfig {
    pub sample_rate: u32,      // 采样率 (48000)
    pub channels: u8,          // 声道数 (2)
    pub bitrate: u32,          // 比特率 (128000)
}

pub struct AudioPacket {
    pub data: Vec<u8>,         // Opus 编码数据
    pub timestamp: u64,
}
```

#### 3.2.4 transport 模块

**职责**: WebSocket 通信

**公开接口**:
```rust
pub struct WebSocketServer {
    // ...
}

impl WebSocketServer {
    /// 创建服务器
    pub fn new(config: ServerConfig) -> Self;

    /// 运行服务器
    pub async fn run(
        &self,
        video_rx: broadcast::Receiver<Vec<EncodedStripe>>,
        audio_rx: broadcast::Receiver<AudioPacket>,
        input_tx: mpsc::Sender<InputEvent>,
    ) -> Result<()>;

    /// 获取连接客户端数
    pub fn client_count(&self) -> usize;
}

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_clients: usize,
}
```

#### 3.2.5 input 模块

**职责**: 输入事件处理

**公开接口**:
```rust
pub struct InputHandler {
    // ...
}

impl InputHandler {
    /// 创建输入处理器
    pub fn new(display: &str) -> Result<Self>;

    /// 处理输入事件
    pub fn handle(&self, event: InputEvent) -> Result<()>;
}

pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: u8, pressed: bool },
    MouseScroll { dx: i32, dy: i32 },
    KeyPress { keysym: u32, pressed: bool },
    Clipboard { data: String },
}
```

---

## 4. 协议规范

### 4.1 WebSocket 消息格式

所有消息均为文本格式，使用逗号分隔字段。

#### 4.1.1 服务端 → 客户端

**视频条纹消息**:
```
s,{y},{height},{base64_jpeg_data}

示例:
s,0,64,/9j/4AAQSkZJRgABAQAA...
s,64,64,/9j/4AAQSkZJRgABA...
```

**音频数据消息**:
```
a,{base64_opus_data}

示例:
a,T3B1c0hlYWQBATgBgLs...
```

**光标消息**:
```
cursor,{json_data}

示例:
cursor,{"x":100,"y":200,"visible":true}
```

**剪贴板消息**:
```
clipboard,{base64_text_data}

示例:
clipboard,SGVsbG8gV29ybGQ=
```

**系统消息**:
```
system,{action},{data}

示例:
system,resolution,1920x1080
system,framerate,30
```

#### 4.1.2 客户端 → 服务端

**鼠标移动**:
```
m,{x},{y}

示例:
m,1024,768
```

**鼠标按键**:
```
b,{button},{pressed}

button: 1=左键, 2=中键, 3=右键
pressed: 0=释放, 1=按下

示例:
b,1,1    # 左键按下
b,1,0    # 左键释放
```

**鼠标滚轮**:
```
w,{dx},{dy}

示例:
w,0,-120  # 向上滚动
w,0,120   # 向下滚动
```

**键盘事件**:
```
k,{keysym},{pressed}

keysym: X11 KeySym 值
pressed: 0=释放, 1=按下

示例:
k,65,1    # 'A' 键按下
k,65,0    # 'A' 键释放
k,65293,1 # Enter 键按下
```

**剪贴板**:
```
c,{base64_text_data}

示例:
c,SGVsbG8gV29ybGQ=
```

**控制命令**:
```
_r,{action}

action: cycleDecoder, toggleKeyboard, etc.

示例:
_r,cycleDecoder
```

### 4.2 协议状态机

```
Client                                  Server
   │                                      │
   │──────── WebSocket Connect ──────────▶│
   │                                      │
   │◀─────── system,resolution,WxH ───────│
   │◀─────── system,framerate,N ──────────│
   │                                      │
   │◀─────── s,0,64,{jpeg} ───────────────│  ┐
   │◀─────── s,64,64,{jpeg} ──────────────│  │ 视频流
   │◀─────── s,128,64,{jpeg} ─────────────│  │
   │◀─────── a,{opus} ────────────────────│  │ 音频流
   │         ...                          │  ┘
   │                                      │
   │──────── m,100,200 ───────────────────▶│  ┐
   │──────── b,1,1 ───────────────────────▶│  │ 输入事件
   │──────── k,65,1 ──────────────────────▶│  ┘
   │                                      │
   │◀─────── cursor,{...} ────────────────│
   │                                      │
   │──────── WebSocket Close ─────────────▶│
   │                                      │
```

---

## 5. 数据结构

### 5.1 核心数据结构

```rust
// ============================================
// 配置相关
// ============================================

/// 全局配置
#[derive(Debug, Clone)]
pub struct Config {
    pub display: String,           // X11 DISPLAY
    pub http_port: u16,            // HTTP 端口
    pub ws_port: u16,              // WebSocket 端口
    pub video: VideoConfig,
    pub audio: AudioConfig,
    pub input: InputConfig,
}

/// 视频配置
#[derive(Debug, Clone)]
pub struct VideoConfig {
    pub fps: u32,                  // 帧率
    pub quality: u8,               // JPEG 质量 (1-100)
    pub stripe_height: u32,        // 条纹高度
    pub max_bandwidth: Option<u32>, // 最大带宽限制 (Kbps)
}

/// 音频配置
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub enabled: bool,
    pub sample_rate: u32,          // 采样率
    pub channels: u8,              // 声道数
    pub bitrate: u32,              // 比特率
}

/// 输入配置
#[derive(Debug, Clone)]
pub struct InputConfig {
    pub enabled: bool,
    pub enable_clipboard: bool,
}

// ============================================
// 视频相关
// ============================================

/// 原始帧
#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,             // BGRA 像素数据
    pub timestamp: Instant,
    pub frame_id: u64,
}

/// 编码后的条纹
#[derive(Debug, Clone)]
pub struct EncodedStripe {
    pub y: u32,                    // Y 坐标
    pub height: u32,               // 高度
    pub data: Vec<u8>,             // JPEG 数据
    pub hash: u64,                 // 内容哈希
    pub is_keyframe: bool,
}

/// 条纹状态 (用于变化检测)
#[derive(Debug)]
struct StripeState {
    hash: u64,
    last_sent: Instant,
    send_count: u32,
}

// ============================================
// 音频相关
// ============================================

/// 音频包
#[derive(Debug, Clone)]
pub struct AudioPacket {
    pub data: Vec<u8>,             // Opus 编码数据
    pub timestamp: u64,            // 时间戳
    pub sequence: u32,             // 序列号
}

// ============================================
// 输入相关
// ============================================

/// 输入事件
#[derive(Debug, Clone)]
pub enum InputEvent {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    MouseScroll {
        dx: i32,
        dy: i32,
    },
    KeyPress {
        keysym: u32,
        pressed: bool,
    },
    Clipboard {
        data: String,
        mime_type: String,
    },
}

/// 鼠标按键
#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left = 1,
    Middle = 2,
    Right = 3,
    ScrollUp = 4,
    ScrollDown = 5,
}

// ============================================
// 客户端相关
// ============================================

/// 客户端状态
#[derive(Debug)]
pub struct ClientState {
    pub id: u64,
    pub addr: SocketAddr,
    pub connected_at: Instant,
    pub last_activity: Instant,
    pub bytes_sent: u64,
    pub frames_sent: u64,
}

/// 服务器统计
#[derive(Debug, Default)]
pub struct ServerStats {
    pub clients_connected: usize,
    pub total_connections: u64,
    pub bytes_sent: u64,
    pub frames_encoded: u64,
    pub current_fps: f32,
    pub current_bandwidth: u64,    // bytes/sec
}
```

### 5.2 错误类型

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SelkiesError {
    // 捕获错误
    #[error("Screen capture failed: {0}")]
    CaptureError(String),

    #[error("No display found")]
    NoDisplay,

    // 编码错误
    #[error("JPEG encoding failed: {0}")]
    JpegEncodeError(String),

    #[error("Opus encoding failed: {0}")]
    OpusEncodeError(String),

    // 输入错误
    #[error("Input injection failed: {0}")]
    InputError(String),

    #[error("Invalid keysym: {0}")]
    InvalidKeysym(u32),

    // 传输错误
    #[error("WebSocket error: {0}")]
    WebSocketError(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    // 配置错误
    #[error("Configuration error: {0}")]
    ConfigError(String),

    // IO 错误
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SelkiesError>;
```

---

## 6. 关键算法

### 6.1 条纹变化检测算法

```rust
/// 条纹变化检测器
pub struct ChangeDetector {
    stripe_hashes: Vec<u64>,
    stripe_last_sent: Vec<Instant>,
    force_refresh_interval: Duration,
}

impl ChangeDetector {
    pub fn new(num_stripes: usize) -> Self {
        Self {
            stripe_hashes: vec![0; num_stripes],
            stripe_last_sent: vec![Instant::now(); num_stripes],
            force_refresh_interval: Duration::from_secs(5),
        }
    }

    /// 检测条纹是否变化
    ///
    /// 返回: (changed, should_send)
    /// - changed: 内容是否变化
    /// - should_send: 是否应该发送 (考虑强制刷新)
    pub fn check(&mut self, stripe_index: usize, data: &[u8]) -> (bool, bool) {
        let hash = xxh3_64(data);
        let old_hash = self.stripe_hashes[stripe_index];
        let last_sent = self.stripe_last_sent[stripe_index];

        let changed = hash != old_hash;
        let force_refresh = last_sent.elapsed() > self.force_refresh_interval;
        let should_send = changed || force_refresh;

        if should_send {
            self.stripe_hashes[stripe_index] = hash;
            self.stripe_last_sent[stripe_index] = Instant::now();
        }

        (changed, should_send)
    }

    /// 强制全帧刷新
    pub fn force_full_refresh(&mut self) {
        self.stripe_hashes.fill(0);
    }
}
```

### 6.2 帧率控制算法

```rust
/// 帧率控制器
pub struct FrameRateController {
    target_fps: u32,
    frame_duration: Duration,
    last_frame: Instant,
    frame_times: VecDeque<Duration>,
    max_samples: usize,
}

impl FrameRateController {
    pub fn new(target_fps: u32) -> Self {
        Self {
            target_fps,
            frame_duration: Duration::from_micros(1_000_000 / target_fps as u64),
            last_frame: Instant::now(),
            frame_times: VecDeque::with_capacity(60),
            max_samples: 60,
        }
    }

    /// 等待下一帧时间
    pub async fn wait_for_next_frame(&mut self) {
        let elapsed = self.last_frame.elapsed();

        if elapsed < self.frame_duration {
            tokio::time::sleep(self.frame_duration - elapsed).await;
        }

        // 记录帧时间
        let actual_duration = self.last_frame.elapsed();
        self.frame_times.push_back(actual_duration);
        if self.frame_times.len() > self.max_samples {
            self.frame_times.pop_front();
        }

        self.last_frame = Instant::now();
    }

    /// 获取当前实际帧率
    pub fn current_fps(&self) -> f32 {
        if self.frame_times.is_empty() {
            return self.target_fps as f32;
        }

        let avg_duration: Duration = self.frame_times.iter().sum::<Duration>()
            / self.frame_times.len() as u32;

        1.0 / avg_duration.as_secs_f32()
    }

    /// 动态调整帧率
    pub fn adjust_fps(&mut self, new_fps: u32) {
        self.target_fps = new_fps.clamp(1, 60);
        self.frame_duration = Duration::from_micros(1_000_000 / self.target_fps as u64);
    }
}
```

### 6.3 带宽自适应算法

```rust
/// 带宽自适应控制器
pub struct BandwidthController {
    target_bandwidth: u64,      // 目标带宽 (bytes/sec)
    current_bandwidth: u64,     // 当前带宽
    quality: u8,                // JPEG 质量
    min_quality: u8,
    max_quality: u8,
    adjustment_interval: Duration,
    last_adjustment: Instant,
}

impl BandwidthController {
    pub fn new(target_bandwidth: u64) -> Self {
        Self {
            target_bandwidth,
            current_bandwidth: 0,
            quality: 80,
            min_quality: 30,
            max_quality: 95,
            adjustment_interval: Duration::from_secs(1),
            last_adjustment: Instant::now(),
        }
    }

    /// 记录发送的数据
    pub fn record_sent(&mut self, bytes: u64) {
        self.current_bandwidth += bytes;
    }

    /// 检查并调整质量
    pub fn check_and_adjust(&mut self) -> Option<u8> {
        if self.last_adjustment.elapsed() < self.adjustment_interval {
            return None;
        }

        let bandwidth_ratio = self.current_bandwidth as f32 / self.target_bandwidth as f32;

        let new_quality = if bandwidth_ratio > 1.2 {
            // 带宽超出 20%，降低质量
            (self.quality as f32 * 0.9) as u8
        } else if bandwidth_ratio < 0.8 && self.quality < self.max_quality {
            // 带宽有余量，提高质量
            (self.quality as f32 * 1.05) as u8
        } else {
            self.quality
        };

        let new_quality = new_quality.clamp(self.min_quality, self.max_quality);

        self.current_bandwidth = 0;
        self.last_adjustment = Instant::now();

        if new_quality != self.quality {
            self.quality = new_quality;
            Some(new_quality)
        } else {
            None
        }
    }
}
```

---

## 7. 配置说明

### 7.1 命令行参数

```
selkies-core [OPTIONS]

OPTIONS:
    -d, --display <DISPLAY>     X11 display [default: :1]
    -p, --port <PORT>           WebSocket port [default: 8082]
    -h, --http-port <PORT>      HTTP port for static files [default: 3000]

    --fps <FPS>                 Target frame rate [default: 30]
    --quality <QUALITY>         JPEG quality 1-100 [default: 80]
    --stripe-height <HEIGHT>    Stripe height in pixels [default: 64]

    --audio                     Enable audio streaming
    --audio-bitrate <BITRATE>   Audio bitrate [default: 128000]

    --no-input                  Disable input handling
    --no-clipboard              Disable clipboard sync

    --max-clients <N>           Max concurrent clients [default: 10]
    --bandwidth <KBPS>          Target bandwidth limit in Kbps

    -c, --config <FILE>         Config file path
    -v, --verbose               Verbose logging
    --help                      Print help
    --version                   Print version
```

### 7.2 配置文件格式 (TOML)

```toml
# /etc/selkies-core/config.toml

[server]
display = ":1"
http_port = 3000
ws_port = 8082
max_clients = 10

[video]
fps = 30
quality = 80
stripe_height = 64
# max_bandwidth = 5000  # Kbps, optional

[audio]
enabled = true
sample_rate = 48000
channels = 2
bitrate = 128000

[input]
enabled = true
enable_clipboard = true

[logging]
level = "info"  # trace, debug, info, warn, error
```

### 7.3 环境变量

| 变量名 | 描述 | 默认值 |
|--------|------|--------|
| `DISPLAY` | X11 display | `:1` |
| `SELKIES_PORT` | WebSocket 端口 | `8082` |
| `SELKIES_HTTP_PORT` | HTTP 端口 | `3000` |
| `SELKIES_FPS` | 帧率 | `30` |
| `SELKIES_QUALITY` | JPEG 质量 | `80` |
| `SELKIES_AUDIO` | 启用音频 | `false` |
| `SELKIES_LOG_LEVEL` | 日志级别 | `info` |

---

## 8. 部署指南

### 8.1 系统依赖

**Debian/Ubuntu:**
```bash
apt-get install -y \
    xvfb \
    x11-utils \
    pulseaudio \
    libjpeg-turbo8 \
    libopus0
```

**Arch Linux:**
```bash
pacman -S \
    xorg-server-xvfb \
    xorg-xauth \
    pulseaudio \
    libjpeg-turbo \
    opus
```

### 8.2 安装步骤

```bash
# 1. 下载二进制
curl -L https://github.com/example/selkies-core/releases/latest/download/selkies-core-linux-amd64 \
    -o /usr/local/bin/selkies-core
chmod +x /usr/local/bin/selkies-core

# 2. 创建配置目录
mkdir -p /etc/selkies-core
mkdir -p /var/lib/selkies-core/www

# 3. 复制前端资源
cp -r /path/to/selkies-dashboard/* /var/lib/selkies-core/www/

# 4. 创建配置文件
cat > /etc/selkies-core/config.toml << 'EOF'
[server]
display = ":1"
http_port = 3000
ws_port = 8082

[video]
fps = 30
quality = 80

[audio]
enabled = true
EOF

# 5. 创建 systemd 服务
cat > /etc/systemd/system/selkies-core.service << 'EOF'
[Unit]
Description=Selkies Core Streaming Service
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/selkies-core -c /etc/selkies-core/config.toml
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

# 6. 启动服务
systemctl daemon-reload
systemctl enable selkies-core
systemctl start selkies-core
```

### 8.3 与 Xvfb 配合使用

```bash
# 启动虚拟 X 服务器
Xvfb :1 -screen 0 1920x1080x24 &

# 设置 DISPLAY
export DISPLAY=:1

# 启动窗口管理器
openbox &

# 启动 selkies-core
selkies-core --display :1 --fps 30
```

### 8.4 Docker 部署 (可选)

```dockerfile
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    xvfb \
    openbox \
    pulseaudio \
    libjpeg-turbo8 \
    libopus0 \
    && rm -rf /var/lib/apt/lists/*

COPY selkies-core /usr/local/bin/
COPY www /var/lib/selkies-core/www/

EXPOSE 3000 8082

CMD ["selkies-core", "--display", ":1", "--fps", "30"]
```

---

## 9. 测试策略

### 9.1 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stripe_encoder() {
        let config = EncoderConfig {
            quality: 80,
            stripe_height: 64,
        };
        let mut encoder = StripeEncoder::new(config).unwrap();

        let frame = Frame {
            width: 1920,
            height: 1080,
            data: vec![0u8; 1920 * 1080 * 4],
            timestamp: Instant::now(),
            frame_id: 0,
        };

        let stripes = encoder.encode(&frame).unwrap();

        // 1080 / 64 = 16.875 -> 17 stripes
        assert!(stripes.len() <= 17);
    }

    #[test]
    fn test_change_detection() {
        let mut detector = ChangeDetector::new(10);

        let data1 = vec![0u8; 1000];
        let data2 = vec![1u8; 1000];

        // 首次应该发送
        let (changed, should_send) = detector.check(0, &data1);
        assert!(should_send);

        // 相同数据不应发送
        let (changed, should_send) = detector.check(0, &data1);
        assert!(!changed);
        assert!(!should_send);

        // 不同数据应发送
        let (changed, should_send) = detector.check(0, &data2);
        assert!(changed);
        assert!(should_send);
    }

    #[test]
    fn test_input_parser() {
        let event = InputEvent::parse("m,100,200").unwrap();
        assert!(matches!(event, InputEvent::MouseMove { x: 100, y: 200 }));

        let event = InputEvent::parse("k,65,1").unwrap();
        assert!(matches!(event, InputEvent::KeyPress { keysym: 65, pressed: true }));
    }
}
```

### 9.2 集成测试

```rust
#[tokio::test]
async fn test_websocket_connection() {
    // 启动测试服务器
    let server = WebSocketServer::new(ServerConfig {
        host: "127.0.0.1".into(),
        port: 0, // 随机端口
        max_clients: 1,
    });

    let addr = server.local_addr();

    tokio::spawn(async move {
        server.run(/* ... */).await
    });

    // 连接客户端
    let (mut ws, _) = tokio_tungstenite::connect_async(
        format!("ws://{}", addr)
    ).await.unwrap();

    // 发送输入
    ws.send(Message::Text("m,100,200".into())).await.unwrap();

    // 接收视频帧
    let msg = ws.next().await.unwrap().unwrap();
    assert!(msg.is_text());
    assert!(msg.to_text().unwrap().starts_with("s,"));
}
```

### 9.3 性能基准测试

```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn benchmark_jpeg_encoding(c: &mut Criterion) {
    let frame = Frame {
        width: 1920,
        height: 1080,
        data: vec![128u8; 1920 * 1080 * 4],
        timestamp: Instant::now(),
        frame_id: 0,
    };

    let config = EncoderConfig {
        quality: 80,
        stripe_height: 64,
    };
    let mut encoder = StripeEncoder::new(config).unwrap();

    c.bench_function("encode_1080p_frame", |b| {
        b.iter(|| encoder.encode(&frame))
    });
}

fn benchmark_change_detection(c: &mut Criterion) {
    let data = vec![0u8; 1920 * 64 * 4];
    let mut detector = ChangeDetector::new(17);

    c.bench_function("xxhash_stripe", |b| {
        b.iter(|| detector.check(0, &data))
    });
}

criterion_group!(benches, benchmark_jpeg_encoding, benchmark_change_detection);
criterion_main!(benches);
```

---

## 10. 实施计划

### 10.1 里程碑

| 阶段 | 内容 | 预计时间 |
|------|------|----------|
| **M1: 基础框架** | 项目结构、配置、日志 | 1 周 |
| **M2: 屏幕捕获** | xcap 集成、帧捕获 | 1 周 |
| **M3: 视频编码** | 条纹分割、JPEG 编码、变化检测 | 1 周 |
| **M4: WebSocket** | 服务器、协议实现 | 1 周 |
| **M5: 输入处理** | 协议解析、X11 XTest | 1 周 |
| **M6: 集成测试** | 与前端联调、性能优化 | 1 周 |
| **M7: 音频支持** | PulseAudio 捕获、Opus 编码 | 1 周 (可选) |
| **总计** | | **6-7 周** |

### 10.2 任务分解

#### M1: 基础框架 (Week 1)
- [ ] 创建 Cargo 项目
- [ ] 实现 CLI 参数解析 (clap)
- [ ] 实现配置文件加载 (toml)
- [ ] 设置日志系统 (tracing)
- [ ] 定义错误类型
- [ ] 编写基础 README

#### M2: 屏幕捕获 (Week 2)
- [ ] 集成 xcap 库
- [ ] 实现 X11Capturer
- [ ] 实现帧率控制
- [ ] 编写单元测试

#### M3: 视频编码 (Week 3)
- [ ] 集成 turbojpeg
- [ ] 实现条纹分割
- [ ] 实现 XXHash 变化检测
- [ ] 实现 StripeEncoder
- [ ] 性能基准测试

#### M4: WebSocket (Week 4)
- [ ] 实现 WebSocket 服务器
- [ ] 实现 Selkies 协议编码
- [ ] 实现多客户端广播
- [ ] 实现连接管理

#### M5: 输入处理 (Week 5)
- [ ] 实现协议解析器
- [ ] 集成 x11rb XTest
- [ ] 实现鼠标事件
- [ ] 实现键盘事件
- [ ] 实现剪贴板同步

#### M6: 集成测试 (Week 6)
- [ ] 与 Selkies 前端联调
- [ ] 性能优化
- [ ] 内存优化
- [ ] 编写部署文档
- [ ] 创建 Release

---

## 11. 附录

### 11.1 X11 KeySym 常用映射

```rust
pub const XK_BackSpace: u32 = 0xff08;
pub const XK_Tab: u32 = 0xff09;
pub const XK_Return: u32 = 0xff0d;
pub const XK_Escape: u32 = 0xff1b;
pub const XK_Delete: u32 = 0xffff;
pub const XK_Home: u32 = 0xff50;
pub const XK_Left: u32 = 0xff51;
pub const XK_Up: u32 = 0xff52;
pub const XK_Right: u32 = 0xff53;
pub const XK_Down: u32 = 0xff54;
pub const XK_End: u32 = 0xff57;
pub const XK_Insert: u32 = 0xff63;
pub const XK_F1: u32 = 0xffbe;
pub const XK_F12: u32 = 0xffc9;
pub const XK_Shift_L: u32 = 0xffe1;
pub const XK_Control_L: u32 = 0xffe3;
pub const XK_Alt_L: u32 = 0xffe9;
pub const XK_Super_L: u32 = 0xffeb;
```

### 11.2 JPEG 质量与文件大小参考

| 质量 | 1080p 条纹大小 | 压缩率 |
|------|----------------|--------|
| 95 | ~80 KB | 3:1 |
| 80 | ~30 KB | 8:1 |
| 60 | ~15 KB | 16:1 |
| 40 | ~8 KB | 30:1 |

### 11.3 参考资料

- [Selkies 项目](https://github.com/selkies-project/selkies)
- [pixelflux](https://github.com/linuxserver/pixelflux)
- [xcap 库](https://github.com/nashaofu/xcap)
- [turbojpeg-rs](https://github.com/nickelc/turbojpeg-rs)
- [x11rb](https://github.com/psychon/x11rb)
- [tokio-tungstenite](https://github.com/snapview/tokio-tungstenite)

---

*文档版本: 1.0*
*最后更新: 2026-01-17*
