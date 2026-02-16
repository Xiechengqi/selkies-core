# iVnc 协议规范

## 1. 概述

本文档定义了 iVnc 服务端与浏览器客户端之间的通信协议。iVnc 使用 WebRTC 进行低延迟视频/音频流传输，通过 DataChannel 传输输入事件和控制消息。

传输层基于 str0m Sans-I/O WebRTC 库，所有媒体和数据通道均通过 ICE-TCP 传输（RFC 4571 帧格式），HTTP/WebSocket 信令/ICE-TCP 共享同一端口。

## 2. 连接建立

### 2.1 信令端点

```
ws://{host}:{port}/webrtc
wss://{host}:{port}/webrtc  (TLS)
```

默认端口：8008

### 2.2 连接流程

```
Client                                          Server
   │                                               │
   │─────────── HTTP GET /webrtc ──────────────────▶│
   │◀────────── WebSocket Upgrade ─────────────────│
   │                                               │
   │─────────── SDP Offer (JSON) ──────────────────▶│
   │                                               │
   │◀────────── SDP Answer (JSON) ─────────────────│
   │                                               │
   │═══════════ ICE-TCP 连接 (同端口) ═════════════│
   │                                               │
   │◀────────── DTLS/SRTP 握手 ────────────────────│
   │                                               │
   │◀══════════ Video RTP (H.264/VP8) ═════════════│
   │◀══════════ Audio RTP (Opus) ══════════════════│
   │◄─────────► DataChannel (SCTP/DTLS) ──────────►│
   │                                               │
```

1. 客户端通过 WebSocket 连接到 `/webrtc` 信令端点
2. 客户端发送 SDP Offer（包含 ICE-TCP candidate）
3. 服务端返回 SDP Answer（包含 ICE-lite TCP passive candidate）
4. 浏览器通过同端口建立 ICE-TCP 连接
5. DTLS/SRTP 握手完成后，开始接收视频和音频 RTP 流
6. DataChannel 打开后，用于双向输入事件和控制消息

### 2.3 同端口复用

HTTP、WebSocket 信令和 ICE-TCP 共享同一端口（默认 8008）。服务端通过 TCP 连接的首字节自动区分协议类型：

- HTTP 方法首字母（`G`/`P`/`H`/`D`/`O`/`C`）→ Axum HTTP 路由
- 其他字节 → ICE-TCP 数据包 → SessionManager

## 3. WebRTC 信令协议

### 3.1 信令消息格式

WebRTC 信令使用 JSON 格式通过 WebSocket 传输。

#### 3.1.1 Offer 消息

客户端发送 SDP Offer：

```json
{
  "type": "offer",
  "sdp": "v=0\r\no=- ..."
}
```

#### 3.1.2 Answer 消息

服务端返回 SDP Answer：

```json
{
  "type": "answer",
  "sdp": "v=0\r\no=- ...",
  "session_id": "session-abc123"
}
```

SDP Answer 中包含 ICE-lite TCP passive candidate，指向同一端口。

## 4. DataChannel 消息格式

### 4.1 通用格式

所有 DataChannel 文本消息均为 UTF-8，使用逗号 `,` 分隔字段：

```
{message_type},{field1},{field2},...
```

二进制消息用于文件上传。

## 5. 服务端消息 (Server → Client via DataChannel)

### 5.1 光标消息 `cursor`

传输鼠标光标状态变化。

**格式:**
```
cursor,{json_data}
```

**JSON 结构:**
```json
{
    "override": "default"
}
```

`override` 值为 CSS cursor 名称：`default`, `pointer`, `text`, `move`, `none` 等。

**示例:**
```
cursor,{"override":"text"}
cursor,{"override":"none"}
```

### 5.2 剪贴板消息 `clipboard`

传输远程应用的剪贴板内容到浏览器。

**格式:**
```
clipboard,{base64_text_data}
```

**示例:**
```
clipboard,SGVsbG8gV29ybGQh
```

### 5.3 任务栏消息 `taskbar`

传输窗口列表信息。

**格式:**
```
taskbar,{json_data}
```

**JSON 结构:**
```json
{
    "windows": [
        {
            "id": 0,
            "title": "Terminal",
            "app_id": "org.gnome.Terminal",
            "display_name": "Terminal",
            "focused": true
        },
        {
            "id": 1,
            "title": "Files",
            "app_id": "org.gnome.Nautilus",
            "display_name": "Files",
            "focused": false
        }
    ]
}
```

### 5.4 统计消息 `stats`

传输性能统计信息（每秒发送一次）。

**格式:**
```
stats,{json_data}
```

**JSON 结构:**
```json
{
    "fps": 30.0,
    "bandwidth": 2500000,
    "total_frames": 1800,
    "total_bytes": 45000000
}
```

### 5.5 Ping 消息

服务端定期发送 keepalive。

**格式:**
```
ping
```

### 5.6 Pong 消息

响应客户端的 ping。

**格式:**
```
pong
```

## 6. 客户端消息 (Client → Server via DataChannel)

### 6.1 鼠标移动 `m`

**格式:**
```
m,{x},{y},{buttonMask},{unused}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| x | i32 | X 坐标 (像素) |
| y | i32 | Y 坐标 (像素) |
| buttonMask | u32 | 按键位掩码 (bit0=左, bit1=中, bit2=右) |
| unused | i32 | 保留字段 |

buttonMask 变化时自动合成按键按下/释放事件。

**示例:**
```
m,512,384,0,0
m,512,384,1,0    # 左键按下（buttonMask bit0 = 1）
m,600,400,1,0    # 拖拽中
m,600,400,0,0    # 左键释放
```

### 6.2 鼠标按键 `b`

**格式:**
```
b,{button},{pressed}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| button | u8 | 按键编号 (0=左, 1=中, 2=右) |
| pressed | u8 | 状态 (0=释放, 1=按下) |

**示例:**
```
b,0,1    # 左键按下
b,0,0    # 左键释放
```

### 6.3 鼠标滚轮 `w`

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
```

### 6.4 键盘事件 `k`

**格式:**
```
k,{keysym},{pressed}
```

**字段:**
| 字段 | 类型 | 描述 |
|------|------|------|
| keysym | u32 | X11 KeySym 值 |
| pressed | u8 | 状态 (0=释放, 1=按下) |

服务端将 X11 keysym 转换为 xkb keycode（evdev + 8）注入 Wayland 合成器。

**常用 KeySym 值:**
| 键 | KeySym (十六进制) |
|----|-------------------|
| a-z | 0x61-0x7A |
| A-Z | 0x41-0x5A |
| 0-9 | 0x30-0x39 |
| Space | 0x20 |
| Enter | 0xFF0D |
| Backspace | 0xFF08 |
| Tab | 0xFF09 |
| Escape | 0xFF1B |
| Shift_L/R | 0xFFE1/0xFFE2 |
| Control_L/R | 0xFFE3/0xFFE4 |
| Alt_L/R | 0xFFE9/0xFFEA |
| Super_L/R | 0xFFEB/0xFFEC |
| F1-F12 | 0xFFBE-0xFFC9 |
| Arrows | 0xFF51-0xFF54 |

**示例:**
```
k,97,1     # 'a' 按下
k,97,0     # 'a' 释放
k,65507,1  # Ctrl 按下
k,97,1     # 'a' 按下 (Ctrl+A)
k,97,0     # 'a' 释放
k,65507,0  # Ctrl 释放
```

### 6.5 文本输入 `t`

通过 IME 提交的文本，使用 zwp_text_input_v3 协议注入。

**格式:**
```
t,{text}
```

**示例:**
```
t,你好世界
```

### 6.6 剪贴板 `cw`

浏览器剪贴板内容发送到远程应用。

**格式:**
```
cw,{base64_text_data}
```

**示例:**
```
cw,SGVsbG8gV29ybGQh
```

### 6.7 分辨率调整 `r`

**格式:**
```
r,{width}x{height}
```

**示例:**
```
r,1920x1080
r,1280x720
```

### 6.8 键盘重置 `kr`

释放所有修饰键（Shift/Ctrl/Alt/Super），清除粘滞状态。

**格式:**
```
kr
```

### 6.9 窗口焦点 `focus`

切换焦点到指定窗口。

**格式:**
```
focus,{window_id}
```

### 6.10 窗口关闭 `close`

关闭指定窗口。

**格式:**
```
close,{window_id}
```

### 6.11 Pong 响应 `pong`

响应服务端的 ping 消息。

**格式:**
```
pong
```

### 6.12 运行时设置 `SETTINGS`

动态调整服务端参数。

**格式:**
```
SETTINGS,{json_data}
```

### 6.13 客户端统计

**格式:**
```
_f,{fps}                    # 客户端渲染帧率
_l,{latency_ms}             # 客户端延迟
_arg_fps,{fps}              # 请求目标帧率
_stats_video,{json}         # WebRTC 视频统计
_stats_audio,{json}         # WebRTC 音频统计
```

## 7. 消息序列图

### 7.1 正常会话流程

```
Client                                          Server
   │                                               │
   │─────────── WS: SDP Offer ─────────────────────▶│
   │◀────────── WS: SDP Answer ────────────────────│
   │                                               │
   │═══════════ ICE-TCP + DTLS 握手 ═══════════════│
   │                                               │
   │◀══════════ Video RTP (H.264) ═════════════════│
   │◀══════════ Audio RTP (Opus) ══════════════════│
   │                                               │
   │◄──────────► DataChannel Open ─────────────────│
   │                                               │
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │◀────────── cursor,{"override":"default"} ─────│
   │                                               │
   │                 [用户输入]                      │
   │─────────── m,512,384,0,0 ─────────────────────▶│
   │─────────── k,97,1 ────────────────────────────▶│
   │─────────── k,97,0 ────────────────────────────▶│
   │                                               │
   │                 [心跳保活]                      │
   │◀────────── ping ──────────────────────────────│
   │─────────── pong ──────────────────────────────▶│
   │                                               │
   │◀────────── stats,{...} ───────────────────────│
   │                                               │
```

### 7.2 剪贴板同步流程

```
Client                                          Server
   │                                               │
   │           [远程应用复制]                        │
   │                                               │
   │           Wayland client → new_selection()     │
   │           → clipboard_pending_mime (延迟)      │
   │           → request_data_device_client_selection│
   │           → 非阻塞 pipe 读取                   │
   │                                               │
   │◀────────── clipboard,SGVsbG8... ──────────────│
   │                                               │
   │           [浏览器粘贴到远程]                    │
   │                                               │
   │─────────── cw,V29ybGQh... ───────────────────▶│
   │                                               │
   │           → set_data_device_selection()        │
   │           → clipboard_suppress_until (500ms)   │
   │           → Wayland client reads via           │
   │             send_selection()                   │
   │                                               │
```

### 7.3 窗口管理流程

```
Client                                          Server
   │                                               │
   │           [新窗口打开]                         │
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │                                               │
   │           [切换窗口焦点]                       │
   │─────────── focus,1 ───────────────────────────▶│
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │                                               │
   │           [关闭窗口]                           │
   │─────────── close,0 ───────────────────────────▶│
   │◀────────── taskbar,{"windows":[...]} ─────────│
   │                                               │
```

## 8. 错误处理

### 8.1 协议错误

当收到无效 DataChannel 消息时，服务端记录日志但不断开连接：

```
debug!("Session {} DC parse error: {}", session.id, e);
```

### 8.2 连接断开

- 服务端通过 ping/pong 机制检测连接活性（15 秒间隔，45 秒超时）
- TCP 连接关闭时自动清理会话
- 客户端应实现自动重连机制

## 9. HTTP 端点

| 端点 | 方法 | 说明 |
|------|------|------|
| `/` | GET | Web 界面（嵌入式前端资源） |
| `/webrtc` | GET (WS) | WebRTC 信令 WebSocket |
| `/health` | GET | 健康检查（JSON） |
| `/metrics` | GET | Prometheus 指标 |
| `/clients` | GET | 活跃连接列表 |
| `/ui-config` | GET | UI 配置 |
| `/ws-config` | GET | WebSocket 端口配置 |

所有 HTTP 端点支持 Basic Auth（可配置）。

---

*文档版本: 2.0*
*最后更新: 2026-02-16*
