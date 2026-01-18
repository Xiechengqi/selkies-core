# Selkies 协议规范

## 1. 概述

本文档定义了 selkies-core 与浏览器客户端之间的通信协议。selkies-core 支持两种传输模式：

1. **WebRTC 模式（默认）** - 使用 WebRTC 进行低延迟视频流传输，通过 DataChannel 传输输入事件
2. **WebSocket 模式（备用）** - 使用 WebSocket 传输 JPEG 条纹和输入事件

两种模式的输入协议格式兼容，便于客户端在不同模式间切换。

## 2. 连接建立

### 2.1 WebRTC 模式

#### 2.1.1 信令端点

```
ws://{host}:{http_port}/webrtc
wss://{host}:{http_port}/webrtc  (TLS)
```

默认 HTTP 端口：8000

#### 2.1.2 连接流程

1. 客户端连接到 WebRTC 信令 WebSocket
2. 客户端发送 SDP Offer
3. 服务端返回 SDP Answer
4. 交换 ICE Candidates
5. 建立 WebRTC PeerConnection
6. 接收视频轨道（RTP）
7. 打开 DataChannel 用于输入事件

### 2.2 WebSocket 模式（备用）

#### 2.2.1 WebSocket 端点

```
ws://{host}:{ws_port}/
wss://{host}:{ws_port}/  (TLS)
```

默认 WebSocket 端口：8080

### 2.2 连接握手

客户端连接后，服务端会立即发送初始化消息：

```
system,resolution,{width}x{height}
system,framerate,{fps}
system,encoder,jpeg
```

## 3. WebRTC 信令协议

### 3.1 信令消息格式

WebRTC 信令使用 JSON 格式通过 WebSocket 传输。

#### 3.1.1 Offer 消息

客户端发送 SDP Offer：

```json
{
  "type": "offer",
  "sdp": "v=0\r\no=- ...",
  "session_id": "optional-session-id"
}
```

#### 3.1.2 Answer 消息

服务端返回 SDP Answer：

```json
{
  "type": "answer",
  "sdp": "v=0\r\no=- ...",
  "session_id": "session-123"
}
```

#### 3.1.3 ICE Candidate 消息

交换 ICE Candidates：

```json
{
  "type": "ice_candidate",
  "candidate": "candidate:...",
  "sdp_mid": "0",
  "sdp_mline_index": 0,
  "session_id": "session-123"
}
```

### 3.2 DataChannel 输入协议

输入事件通过 WebRTC DataChannel 传输，格式与 WebSocket 模式兼容。

## 4. WebSocket 消息格式

### 4.1 通用格式

所有消息均为 UTF-8 文本，使用逗号 `,` 分隔字段：

```
{message_type},{field1},{field2},...
```

### 4.2 二进制数据编码

二进制数据（如 JPEG、Opus）使用 Base64 编码传输。

## 5. 服务端消息 (Server → Client)

### 5.1 视频条纹消息 `s` (WebSocket 模式)

传输 JPEG 编码的屏幕条纹。

**格式:**
```
s,{y},{height},{base64_jpeg_data}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| y | u32 | 条纹起始 Y 坐标 (像素) |
| height | u32 | 条纹高度 (像素) |
| base64_jpeg_data | string | Base64 编码的 JPEG 图像 |

**示例:**
```
s,0,64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQ...
s,64,64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJ...
s,128,64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwc...
```

**客户端处理:**
```javascript
const [type, y, height, jpegData] = message.split(',');
if (type === 's') {
    const img = new Image();
    img.onload = () => {
        ctx.drawImage(img, 0, parseInt(y), canvas.width, parseInt(height));
    };
    img.src = 'data:image/jpeg;base64,' + jpegData;
}
```

### 4.2 音频消息 `a`

传输 Opus 编码的音频数据。

**格式:**
```
a,{base64_opus_data}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| base64_opus_data | string | Base64 编码的 Opus 音频帧 |

**示例:**
```
a,T3B1c0hlYWQBATgBgLsAAAAAAA...
```

### 4.3 光标消息 `cursor`

传输鼠标光标信息。

**格式:**
```
cursor,{json_data}
```

**JSON 结构:**
```json
{
    "x": 100,
    "y": 200,
    "visible": true,
    "image": "base64_png_data",  // 可选，自定义光标
    "hotspot_x": 0,              // 可选
    "hotspot_y": 0               // 可选
}
```

**示例:**
```
cursor,{"x":512,"y":384,"visible":true}
```

### 4.4 剪贴板消息 `clipboard`

传输剪贴板内容。

**格式 (文本):**
```
clipboard,{base64_text_data}
```

**格式 (大数据分块):**
```
clipboard_start,{mime_type},{total_size}
clipboard_data,{base64_chunk}
clipboard_data,{base64_chunk}
clipboard_finish
```

**示例:**
```
clipboard,SGVsbG8gV29ybGQh
```

### 4.5 系统消息 `system`

传输系统状态和控制信息。

**格式:**
```
system,{action},{data}
```

**动作类型:**

| 动作 | 数据格式 | 描述 |
|------|----------|------|
| resolution | `{width}x{height}` | 屏幕分辨率 |
| framerate | `{fps}` | 当前帧率 |
| encoder | `{encoder_name}` | 编码器名称 |
| bitrate | `{kbps}` | 当前比特率 |
| latency | `{ms}` | 往返延迟 |
| ui_config | `{json}` | UI 配置 |
| reload | (无) | 请求客户端刷新 |

**示例:**
```
system,resolution,1920x1080
system,framerate,30
system,encoder,jpeg
system,reload
system,ui_config,{"version":"1",...}
```

### 4.6 统计消息 `stats`

传输性能统计信息。

**格式:**
```
stats,{json_data}
```

**JSON 结构:**
```json
{
    "fps": 30.0,
    "bandwidth": 2500000,
    "latency": 15,
    "clients": 1,
    "cpu_percent": 5.2,
    "mem_used": 45000000
}
```

### 4.7 Ping 消息 `ping`

用于测量往返延迟。

**格式:**
```
ping,{timestamp}
```

**示例:**
```
ping,1705500000.123
```

## 5. 客户端消息 (Client → Server)

### 5.1 鼠标移动 `m`

**格式:**
```
m,{x},{y}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| x | i32 | X 坐标 (像素) |
| y | i32 | Y 坐标 (像素) |

**示例:**
```
m,512,384
m,1024,768
```

### 5.2 鼠标按键 `b`

**格式:**
```
b,{button},{pressed}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| button | u8 | 按键编号 (1=左, 2=中, 3=右) |
| pressed | u8 | 状态 (0=释放, 1=按下) |

**示例:**
```
b,1,1    # 左键按下
b,1,0    # 左键释放
b,3,1    # 右键按下
```

### 5.3 鼠标滚轮 `w`

**格式:**
```
w,{dx},{dy}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| dx | i32 | 水平滚动量 |
| dy | i32 | 垂直滚动量 (正=向下, 负=向上) |

**示例:**
```
w,0,-120   # 向上滚动
w,0,120    # 向下滚动
w,-120,0   # 向左滚动
```

### 5.4 键盘事件 `k`

**格式:**
```
k,{keysym},{pressed}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| keysym | u32 | X11 KeySym 值 |
| pressed | u8 | 状态 (0=释放, 1=按下) |

**常用 KeySym 值:**
| 键 | KeySym (十进制) | KeySym (十六进制) |
|----|-----------------|-------------------|
| A-Z | 65-90 | 0x41-0x5A |
| a-z | 97-122 | 0x61-0x7A |
| 0-9 | 48-57 | 0x30-0x39 |
| Space | 32 | 0x20 |
| Enter | 65293 | 0xFF0D |
| Backspace | 65288 | 0xFF08 |
| Tab | 65289 | 0xFF09 |
| Escape | 65307 | 0xFF1B |
| Shift_L | 65505 | 0xFFE1 |
| Control_L | 65507 | 0xFFE3 |
| Alt_L | 65513 | 0xFFE9 |
| F1-F12 | 65470-65481 | 0xFFBE-0xFFC9 |
| Left | 65361 | 0xFF51 |
| Up | 65362 | 0xFF52 |
| Right | 65363 | 0xFF53 |
| Down | 65364 | 0xFF54 |

**示例:**
```
k,65,1     # 'A' 按下
k,65,0     # 'A' 释放
k,65507,1  # Ctrl 按下
k,65,1     # 'A' 按下 (Ctrl+A)
k,65,0     # 'A' 释放
k,65507,0  # Ctrl 释放
```

### 5.5 剪贴板 `c`

**格式:**
```
c,{base64_text_data}
```

**示例:**
```
c,SGVsbG8gV29ybGQh
```

### 5.6 Pong 响应 `pong`

响应服务端的 ping 消息。

**格式:**
```
pong,{timestamp}
```

**示例:**
```
pong,1705500000.123
```

### 5.7 控制命令 `_r`

发送控制命令。

**格式:**
```
_r,{action}
```

**动作类型:**
| 动作 | 描述 |
|------|------|
| cycleDecoder | 切换解码器 |
| toggleKeyboard | 切换虚拟键盘 |
| requestFullRefresh | 请求全帧刷新 |
| requestStats | 请求统计信息 |

**示例:**
```
_r,requestFullRefresh
_r,requestStats
```

## 6. 消息序列图

### 6.1 正常会话流程

```
Client                                          Server
   │                                               │
   │─────────── WebSocket Connect ────────────────▶│
   │                                               │
   │◀────────── system,resolution,1920x1080 ──────│
   │◀────────── system,framerate,30 ──────────────│
   │◀────────── system,encoder,jpeg ──────────────│
   │                                               │
   │                 [视频流开始]                   │
   │◀────────── s,0,64,{jpeg} ────────────────────│
   │◀────────── s,64,64,{jpeg} ───────────────────│
   │◀────────── s,128,64,{jpeg} ──────────────────│
   │             ...                               │
   │                                               │
   │                 [用户输入]                     │
   │─────────── m,512,384 ────────────────────────▶│
   │                                               │
   │◀────────── cursor,{"x":512,"y":384} ─────────│
   │                                               │
   │─────────── b,1,1 ────────────────────────────▶│  (点击)
   │─────────── b,1,0 ────────────────────────────▶│
   │                                               │
   │─────────── k,65,1 ───────────────────────────▶│  (按键 'A')
   │─────────── k,65,0 ───────────────────────────▶│
   │                                               │
   │                 [延迟测量]                     │
   │◀────────── ping,1705500000.123 ──────────────│
   │─────────── pong,1705500000.123 ──────────────▶│
   │◀────────── system,latency,15 ────────────────│
   │                                               │
   │─────────── WebSocket Close ──────────────────▶│
   │                                               │
```

### 6.2 剪贴板同步流程

```
Client                                          Server
   │                                               │
   │                 [服务端复制]                   │
   │◀────────── clipboard,SGVsbG8... ─────────────│
   │                                               │
   │                 [客户端粘贴]                   │
   │─────────── c,V29ybGQh... ────────────────────▶│
   │                                               │
   │                 [大文件分块]                   │
   │◀────────── clipboard_start,text/plain,10000 ─│
   │◀────────── clipboard_data,{chunk1} ──────────│
   │◀────────── clipboard_data,{chunk2} ──────────│
   │◀────────── clipboard_data,{chunk3} ──────────│
   │◀────────── clipboard_finish ─────────────────│
   │                                               │
```

## 7. 错误处理

### 7.1 协议错误

当收到无效消息时，服务端会记录日志但不会断开连接：

```rust
// 服务端处理
match parse_message(&msg) {
    Ok(event) => handle_event(event),
    Err(e) => {
        tracing::warn!("Invalid message: {}", e);
        // 继续处理后续消息
    }
}
```

### 7.2 连接断开

客户端应实现自动重连机制：

```javascript
function connect() {
    const ws = new WebSocket(url);

    ws.onclose = () => {
        setTimeout(connect, 1000); // 1秒后重连
    };

    ws.onerror = (err) => {
        console.error('WebSocket error:', err);
        ws.close();
    };
}
```

## 8. 性能建议

### 8.1 消息合并

客户端应合并高频输入事件：

```javascript
let pendingMouseMove = null;

document.addEventListener('mousemove', (e) => {
    pendingMouseMove = { x: e.clientX, y: e.clientY };
});

setInterval(() => {
    if (pendingMouseMove) {
        ws.send(`m,${pendingMouseMove.x},${pendingMouseMove.y}`);
        pendingMouseMove = null;
    }
}, 16); // ~60Hz
```

### 8.2 背压处理

当网络拥塞时，服务端会降低帧率：

```rust
if send_buffer.len() > MAX_BUFFER_SIZE {
    // 跳过当前帧
    frame_rate_controller.skip_frame();
}
```

## 9. 安全考虑

### 9.1 输入验证

服务端必须验证所有输入坐标：

```rust
fn validate_mouse_coords(x: i32, y: i32, width: u32, height: u32) -> bool {
    x >= 0 && y >= 0 && x < width as i32 && y < height as i32
}
```

### 9.2 KeySym 过滤

可选择过滤危险的快捷键：

```rust
const BLOCKED_KEYSYMS: &[u32] = &[
    0xFFEB, // Super_L (可能触发系统快捷键)
    0xFFEC, // Super_R
];
```

## 10. 版本兼容性

| 协议版本 | selkies-core 版本 | 变更 |
|----------|-------------------|------|
| 1.0 | 0.1.x | 初始版本 |

---

*文档版本: 1.0*
*最后更新: 2026-01-17*
