//! Parameter structs and helpers for MCP tools.

use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

// ── Screenshot ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    /// Optional delay in milliseconds before capturing (0-30000)
    #[serde(default)]
    pub delay_ms: Option<u64>,
}

// ── Mouse ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MouseMoveParams {
    /// X coordinate
    pub x: i32,
    /// Y coordinate
    pub y: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MouseClickParams {
    /// Mouse button: "left" (default), "right", or "middle"
    #[serde(default = "default_button")]
    pub button: String,
    /// X coordinate
    pub x: i32,
    /// Y coordinate
    pub y: i32,
    /// Double-click
    #[serde(default)]
    pub double: bool,
}

fn default_button() -> String { "left".into() }

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MouseScrollParams {
    /// Horizontal scroll delta
    #[serde(default)]
    pub dx: i16,
    /// Vertical scroll delta (positive = scroll down)
    pub dy: i16,
}

// ── Keyboard ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyboardTypeParams {
    /// Text to type
    pub text: String,
    /// Press Enter after typing (default: false)
    #[serde(default)]
    pub enter: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyboardTypeMultilineParams {
    /// Lines of text to type (Enter is pressed after each line)
    pub lines: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyboardKeyParams {
    /// Key or combo string, e.g. "Return", "Ctrl+c", "Alt+F4"
    pub key: String,
}

// ── Clipboard ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClipboardWriteParams {
    /// Text to write to the clipboard
    pub text: String,
}

// ── Window ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WindowIdParams {
    /// Window ID (index from list_windows)
    pub window_id: u32,
}
