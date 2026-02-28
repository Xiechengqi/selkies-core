//! MCP (Model Context Protocol) server for iVnc.
//!
//! Exposes desktop control tools (screenshot, mouse, keyboard, clipboard,
//! window management) via the MCP protocol over stdio or Streamable HTTP.

pub mod frame_capture;
pub mod keyboard;
pub mod tools;

use std::sync::Arc;
use rmcp::{
    ErrorData as McpError, ServerHandler, RoleServer, model::*,
    tool, tool_router,
    service::RequestContext,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    handler::server::tool::ToolCallContext,
};
use base64::Engine;
use crate::web::SharedState;
use crate::input::{InputEvent, InputEventData};
use tools::*;

#[derive(Clone)]
pub struct McpServer {
    pub state: Arc<SharedState>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

// Helper methods (not tools)
impl McpServer {
    fn validate_coords(&self, x: i32, y: i32) -> Result<(), McpError> {
        let (w, h) = self.state.display_size();
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return Err(McpError::invalid_params(
                format!("coordinates ({}, {}) out of bounds ({}x{})", x, y, w, h),
                None,
            ));
        }
        Ok(())
    }

    fn send_key(&self, keysym: u32, pressed: bool) {
        let _ = self.state.input_sender.send(InputEventData {
            event_type: InputEvent::Keyboard,
            keysym,
            key_pressed: pressed,
            ..Default::default()
        });
    }

    async fn type_char(&self, c: char) {
        let needs_shift = keyboard::char_needs_shift(c);
        let base = if needs_shift { keyboard::get_unshifted_char(c) } else { c };
        let sym = keyboard::char_to_keysym(base);
        if needs_shift { self.send_key(0xffe1, true); }
        self.send_key(sym, true);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.send_key(sym, false);
        if needs_shift { self.send_key(0xffe1, false); }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }

    fn send_text_input(&self, text: &str) {
        let _ = self.state.input_sender.send(InputEventData {
            event_type: InputEvent::TextInput,
            text: text.to_string(),
            ..Default::default()
        });
    }

    fn text_is_ascii_typeable(text: &str) -> bool {
        text.chars().all(|c| c.is_ascii() && !c.is_ascii_control())
    }
}

#[tool_router]
impl McpServer {
    #[tool(description = "Capture the current desktop as a JPEG image. Use delay_ms to wait for UI updates before capturing.")]
    pub async fn screenshot(
        &self,
        Parameters(params): Parameters<ScreenshotParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(delay) = params.delay_ms {
            let delay = delay.min(30000);
            if delay > 0 { tokio::time::sleep(std::time::Duration::from_millis(delay)).await; }
        }
        let (w, h, pixels) = frame_capture::capture_frame(&self.state).await
            .map_err(|e| McpError::internal_error(e, None))?;
        let b64 = frame_capture::xrgb_to_jpeg_base64(w, h, &pixels, 80, 800_000)
            .map_err(|e| McpError::internal_error(e, None))?;
        Ok(CallToolResult::success(vec![Content::image(b64, "image/jpeg")]))
    }

    #[tool(description = "Move the mouse cursor to the specified coordinates.")]
    pub async fn mouse_move(
        &self,
        Parameters(params): Parameters<MouseMoveParams>,
    ) -> Result<CallToolResult, McpError> {
        self.validate_coords(params.x, params.y)?;
        let _ = self.state.input_sender.send(InputEventData {
            event_type: InputEvent::MouseMove, mouse_x: params.x, mouse_y: params.y, ..Default::default()
        });
        Ok(CallToolResult::success(vec![Content::text(format!("Moved to ({}, {})", params.x, params.y))]))
    }

    #[tool(description = "Click a mouse button at coordinates. Supports left/right/middle and double-click.")]
    pub async fn mouse_click(
        &self,
        Parameters(params): Parameters<MouseClickParams>,
    ) -> Result<CallToolResult, McpError> {
        self.validate_coords(params.x, params.y)?;
        // Move cursor to click position first — the compositor button handler
        // uses the pointer's current location, not the event coordinates.
        let _ = self.state.input_sender.send(InputEventData {
            event_type: InputEvent::MouseMove, mouse_x: params.x, mouse_y: params.y, ..Default::default()
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let button: u8 = match params.button.as_str() {
            "left" => 0, "middle" => 1, "right" => 2,
            other => return Err(McpError::invalid_params(format!("unknown button: {}", other), None)),
        };
        let clicks = if params.double { 2 } else { 1 };
        for i in 0..clicks {
            if i > 0 { tokio::time::sleep(std::time::Duration::from_millis(50)).await; }
            let _ = self.state.input_sender.send(InputEventData {
                event_type: InputEvent::MouseButton, mouse_x: params.x, mouse_y: params.y,
                mouse_button: button, button_pressed: true, ..Default::default()
            });
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let _ = self.state.input_sender.send(InputEventData {
                event_type: InputEvent::MouseButton, mouse_x: params.x, mouse_y: params.y,
                mouse_button: button, button_pressed: false, ..Default::default()
            });
        }
        let action = if params.double { "Double-clicked" } else { "Clicked" };
        Ok(CallToolResult::success(vec![Content::text(format!("{} {} at ({}, {})", action, params.button, params.x, params.y))]))
    }

    #[tool(description = "Scroll the mouse wheel. Positive dy scrolls down, negative scrolls up.")]
    pub async fn mouse_scroll(
        &self,
        Parameters(params): Parameters<MouseScrollParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = self.state.input_sender.send(InputEventData {
            event_type: InputEvent::MouseWheel, wheel_delta_x: params.dx, wheel_delta_y: params.dy, ..Default::default()
        });
        Ok(CallToolResult::success(vec![Content::text(format!("Scrolled dx={} dy={}", params.dx, params.dy))]))
    }

    #[tool(description = "Type text using the keyboard. Supports ASCII and non-ASCII (CJK, emoji, etc.) text. Non-ASCII text is sent via IME/text input.")]
    pub async fn keyboard_type(
        &self,
        Parameters(params): Parameters<KeyboardTypeParams>,
    ) -> Result<CallToolResult, McpError> {
        if Self::text_is_ascii_typeable(&params.text) {
            for c in params.text.chars() { self.type_char(c).await; }
        } else {
            self.send_text_input(&params.text);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if params.enter {
            self.send_key(0xff0d, true);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            self.send_key(0xff0d, false);
        }
        Ok(CallToolResult::success(vec![Content::text(
            format!("Typed {} chars{}", params.text.chars().count(), if params.enter { " + Enter" } else { "" }),
        )]))
    }

    #[tool(description = "Type multiple lines of text. Enter is pressed after each line. Supports non-ASCII (CJK, emoji, etc.) text via IME.")]
    pub async fn keyboard_type_multiline(
        &self,
        Parameters(params): Parameters<KeyboardTypeMultilineParams>,
    ) -> Result<CallToolResult, McpError> {
        let count = params.lines.len();
        for (i, line) in params.lines.iter().enumerate() {
            if Self::text_is_ascii_typeable(line) {
                for c in line.chars() { self.type_char(c).await; }
            } else {
                self.send_text_input(line);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            self.send_key(0xff0d, true);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            self.send_key(0xff0d, false);
            if i < count - 1 { tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
        }
        Ok(CallToolResult::success(vec![Content::text(format!("Typed {} lines", count))]))
    }

    #[tool(description = "Press a key or key combination. Use '+' for combos: 'Ctrl+c', 'Alt+F4', 'Ctrl+Shift+t'. Single keys: 'Return', 'Escape', 'Tab', 'F1'-'F12', arrows, etc.")]
    pub async fn keyboard_key(
        &self,
        Parameters(params): Parameters<KeyboardKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        let (modifiers, main_sym) = keyboard::parse_key_combo(&params.key)
            .map_err(|e| McpError::invalid_params(e, None))?;
        for &m in &modifiers {
            self.send_key(m, true);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        self.send_key(main_sym, true);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        self.send_key(main_sym, false);
        for &m in modifiers.iter().rev() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            self.send_key(m, false);
        }
        Ok(CallToolResult::success(vec![Content::text(format!("Pressed {}", params.key))]))
    }

    #[tool(description = "Read the current clipboard text content.")]
    pub async fn clipboard_read(&self) -> Result<CallToolResult, McpError> {
        let clip = self.state.clipboard.lock().unwrap().clone();
        match clip {
            Some(b64) => {
                let decoded = base64::engine::general_purpose::STANDARD.decode(&b64)
                    .map_err(|e| McpError::internal_error(format!("base64 decode: {}", e), None))?;
                let text = String::from_utf8_lossy(&decoded).into_owned();
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            None => Ok(CallToolResult::success(vec![Content::text("(clipboard empty)")])),
        }
    }

    #[tool(description = "Write text to the clipboard.")]
    pub async fn clipboard_write(
        &self,
        Parameters(params): Parameters<ClipboardWriteParams>,
    ) -> Result<CallToolResult, McpError> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(params.text.as_bytes());
        let _ = self.state.clipboard_incoming_tx.send(b64);
        self.state.clipboard_incoming_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(CallToolResult::success(vec![Content::text("Clipboard updated")]))
    }

    #[tool(description = "Get screen dimensions, FPS, bandwidth, and connection statistics.")]
    pub async fn get_screen_info(&self) -> Result<CallToolResult, McpError> {
        let (w, h) = self.state.display_size();
        let stats = self.state.stats.lock().unwrap().clone();
        let sessions = self.state.webrtc_sessions();
        let uptime = self.state.uptime().as_secs();
        let info = serde_json::json!({
            "width": w, "height": h,
            "fps": format!("{:.1}", stats.fps),
            "bandwidth_bps": stats.bandwidth,
            "webrtc_sessions": sessions,
            "uptime_seconds": uptime,
            "cpu_percent": format!("{:.1}", stats.cpu_percent),
            "mem_bytes": stats.mem_used,
        });
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&info).unwrap(),
        )]))
    }

    #[tool(description = "List all open windows with their IDs, titles, and focus state.")]
    pub async fn list_windows(&self) -> Result<CallToolResult, McpError> {
        let json = self.state.last_taskbar_json.lock().unwrap().clone();
        match json {
            Some(j) => Ok(CallToolResult::success(vec![Content::text(j)])),
            None => Ok(CallToolResult::success(vec![Content::text(r#"{"windows":[]}"#)])),
        }
    }

    #[tool(description = "Focus a window by its ID (from list_windows).")]
    pub async fn window_focus(
        &self,
        Parameters(params): Parameters<WindowIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = self.state.input_sender.send(InputEventData {
            event_type: InputEvent::WindowFocus,
            window_id: params.window_id,
            ..Default::default()
        });
        Ok(CallToolResult::success(vec![Content::text(
            format!("Focused window {}", params.window_id),
        )]))
    }

    #[tool(description = "Close a window by its ID (from list_windows).")]
    pub async fn window_close(
        &self,
        Parameters(params): Parameters<WindowIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let _ = self.state.input_sender.send(InputEventData {
            event_type: InputEvent::WindowClose,
            window_id: params.window_id,
            ..Default::default()
        });
        Ok(CallToolResult::success(vec![Content::text(
            format!("Closed window {}", params.window_id),
        )]))
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability { list_changed: None }),
                ..Default::default()
            },
            server_info: Implementation {
                name: "ivnc-mcp".into(),
                title: None,
                version: env!("CARGO_PKG_VERSION").into(),
                description: Some("iVnc remote desktop MCP server".into()),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "iVnc remote desktop MCP server. Use screenshot to see the desktop, \
                 mouse/keyboard tools to interact, clipboard to read/write text, \
                 and window tools to manage windows.".into(),
            ),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tool_router.list_all()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let ctx = ToolCallContext::new(self, request, context);
        self.tool_router.call(ctx).await
    }
}