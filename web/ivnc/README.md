# iVnc Client API

This document outlines the API for an external dashboard to interact with the iVnc client-side application. Interaction primarily occurs via the standard `window.postMessage` mechanism and by observing specific global variables for statistics.

## Connection Mode

The client connects to the iVnc server via WebRTC. All media (video/audio) is delivered over WebRTC tracks, and all input/control messages are exchanged through a WebRTC DataChannel.

### Connection Flow

1. Client connects to `ws://<host>:<port>/webrtc` for WebSocket signaling
2. Client sends SDP Offer, server returns SDP Answer
3. ICE-TCP connection established on the same port
4. DTLS/SRTP handshake completes
5. Video and audio RTP streams begin
6. DataChannel opens for bidirectional input/control messages

### Authentication

HTTP Basic Auth is supported. When enabled, the browser prompts for credentials before loading the page. The WebSocket signaling connection inherits the HTTP authentication.

## 1. Window Messaging API (Dashboard → Client)

The client listens for messages sent via `window.postMessage`. To ensure security, the client only accepts messages from the same origin (`event.origin === window.location.origin`).

All messages sent to the client should be JavaScript objects with a `type` property indicating the action to perform.

### Supported Messages:

---

**Type:** `setScaleLocally`

*   **Payload:** `{ type: 'setScaleLocally', value: <boolean> }`
*   **Description:** Sets the client-side preference for how video is scaled when using manual resolution.
    *   `true`: Scales the video canvas locally to fit within the container while maintaining the aspect ratio.
    *   `false`: Renders the canvas at the exact manual resolution.
    *   Persisted in `localStorage`.

---

**Type:** `showVirtualKeyboard`

*   **Payload:** `{ type: 'showVirtualKeyboard' }`
*   **Description:** Focuses a hidden input element to bring up the OS virtual keyboard on mobile/touch devices.

---

**Type:** `setManualResolution`

*   **Payload:** `{ type: 'setManualResolution', width: <number>, height: <number> }`
*   **Description:** Switches to manual resolution mode. Sends `r,WIDTHxHEIGHT` to the server via DataChannel. The server rebuilds the GStreamer pipeline and reconfigures all non-dialog Wayland windows to the new size.

---

**Type:** `resetResolutionToWindow`

*   **Payload:** `{ type: 'resetResolutionToWindow' }`
*   **Description:** Reverts to automatic resizing based on the browser window size.

---

**Type:** `settings`

*   **Payload:** `{ type: 'settings', settings: <object> }`
*   **Description:** Applies client-side settings and propagates to the server via DataChannel.
*   **Supported `settings` properties:**
    *   `videoBitRate`: (Number) Target video bitrate in kbps. Sends `vb,VALUE`.
    *   `videoFramerate`: (Number) Target framerate. Sends `_arg_fps,VALUE`.
    *   `audioBitRate`: (Number) Audio bitrate in kbit/s. Sends `ab,VALUE`.
    *   `videoBufferSize`: (Number) Client-side video frame buffer size (0 = immediate).
    *   `debug`: (Boolean) Enable verbose debug logging.

---

**Type:** `clipboardUpdateFromUI`

*   **Payload:** `{ type: 'clipboardUpdateFromUI', text: <string> }`
*   **Description:** Sends text to the server as clipboard content. Base64 encoded and sent as `cw,BASE64_TEXT` via DataChannel. The server calls `set_data_device_selection()` to make it available to Wayland clients.

---

**Type:** `requestFullscreen`

*   **Payload:** `{ type: 'requestFullscreen' }`
*   **Description:** Triggers the browser Fullscreen API on the video container.

## 2. Client State & Statistics (Client → Dashboard)

### Key Global Variables:

*   `window.fps`: (Number) Client-side rendering frames per second.
*   `window.currentAudioBufferSize`: (Number) Audio buffers queued in the playback AudioWorklet.
*   `connectionStat`: (Object) WebRTC connection statistics:
    ```javascript
    {
      connectionStatType: 'webrtc',
      connectionLatency: 0,
      connectionVideoLatency: 0,
      connectionAudioLatency: 0,
      connectionAudioCodecName: 'opus',
      connectionAudioBitrate: 0,
      connectionPacketsReceived: 0,
      connectionPacketsLost: 0,
      connectionBytesReceived: 0,
      connectionBytesSent: 0,
      connectionCodec: 'H264',
      connectionVideoDecoder: 'unknown',
      connectionResolution: '1920x1080',
      connectionFrameRate: 0,
      connectionVideoBitrate: 0,
      connectionAvailableBandwidth: 0
    }
    ```
*   `serverClipboardContent`: (String) Last clipboard content received from the server.

### Messages Sent from Client to Dashboard:

*   **Type:** `clipboardContentUpdate`
    *   **Payload:** `{ type: 'clipboardContentUpdate', text: <string> }`
    *   **Description:** Sent when the client receives new clipboard content from the server via DataChannel `clipboard,{base64}` message.

*   **Type:** `fileUpload`
    *   **Payload:** `{ type: 'fileUpload', payload: <object> }`
    *   **Description:** File upload progress notifications.
    *   **Payload `status` values:**
        *   `'start'`: `{ status: 'start', fileName: <string>, fileSize: <number> }`
        *   `'progress'`: `{ status: 'progress', fileName: <string>, progress: <number (0-100)>, fileSize: <number> }`
        *   `'end'`: `{ status: 'end', fileName: <string>, fileSize: <number> }`
        *   `'error'`: `{ status: 'error', fileName: <string>, message: <string> }`

## 3. DataChannel Protocol

The client communicates with the iVnc server through a WebRTC DataChannel. See [PROTOCOL.md](../../docs/PROTOCOL.md) for the complete protocol specification.

### Key messages sent by the client:

| Message | Description |
|---------|-------------|
| `m,{x},{y},{buttonMask},{0}` | Mouse move with button state |
| `k,{keysym},{pressed}` | Keyboard event (X11 keysym) |
| `w,{dx},{dy}` | Mouse wheel scroll |
| `cw,{base64}` | Clipboard content (browser → remote) |
| `r,{width}x{height}` | Resolution change request |
| `focus,{id}` | Switch window focus |
| `close,{id}` | Close window |
| `kr` | Keyboard reset (release all modifiers) |
| `pong` | Keepalive response |
| `t,{text}` | IME text input |

### Key messages received by the client:

| Message | Description |
|---------|-------------|
| `cursor,{json}` | Cursor style change |
| `clipboard,{base64}` | Clipboard content (remote → browser) |
| `taskbar,{json}` | Window list update |
| `stats,{json}` | Server performance statistics |
| `ping` | Keepalive request |

## 4. Replicating UI Interactions

An external dashboard needs to implement:

1.  **Settings Controls:** Use the `settings` message type for bitrate, framerate, etc.
2.  **Resolution Control:** Use `setManualResolution` / `resetResolutionToWindow`.
3.  **Fullscreen:** Send `requestFullscreen` message.
4.  **Stats Display:** Read `connectionStat`, `window.fps` global variables.
5.  **Clipboard:** Display content from `clipboardContentUpdate`, send changes via `clipboardUpdateFromUI`.
6.  **File Upload:** Dispatch `CustomEvent('requestFileUpload')` on `window` to trigger file input. Listen for `fileUpload` messages for progress.
7.  **Virtual Keyboard:** Send `showVirtualKeyboard` for mobile/touch environments.

Remember to handle the origin check when sending messages.
