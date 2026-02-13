//! Input event types for WebRTC input forwarding
//!
//! Defines the input event data structures used by the data channel
//! and compositor input injection.

/// Input event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    MouseMove,
    MouseButton,
    MouseWheel,
    Keyboard,
    KeyboardReset,
    TextInput,
    Clipboard,
    Ping,
    WindowFocus,
    WindowClose,
}

/// Input event data passed from WebRTC data channel to compositor
#[derive(Debug, Clone)]
pub struct InputEventData {
    pub event_type: InputEvent,
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub mouse_button: u8,
    pub button_pressed: bool,
    pub wheel_delta_x: i16,
    pub wheel_delta_y: i16,
    pub keysym: u32,
    pub key_pressed: bool,
    pub button_mask: u32,
    pub text: String,
    pub timestamp: u64,
    pub window_id: u32,
}

impl Default for InputEventData {
    fn default() -> Self {
        Self {
            event_type: InputEvent::MouseMove,
            mouse_x: 0,
            mouse_y: 0,
            mouse_button: 0,
            button_pressed: false,
            wheel_delta_x: 0,
            wheel_delta_y: 0,
            keysym: 0,
            key_pressed: false,
            button_mask: 0,
            text: String::new(),
            timestamp: 0,
            window_id: 0,
        }
    }
}
