//! WebRTC DataChannel input parsing
//!
//! Provides input event parsing for DataChannel text messages,
//! compatible with the existing WebSocket input protocol.
//! The actual DataChannel I/O is handled by str0m in rtc_session.rs.

#![allow(dead_code)]

use super::WebRTCError;
use crate::input::{InputEvent, InputEventData};

/// Input message parser for DataChannel text protocol.
///
/// This is a stateless parser — the actual DataChannel lifecycle
/// is managed by str0m's event-based API in `rtc_session.rs`.
pub struct InputDataChannel;

impl InputDataChannel {
    /// Parse input text message
    ///
    /// Supports the same protocol as WebSocket:
    /// - Mouse move: `m,x,y` or `m,x,y,buttons`
    /// - Relative mouse: `m2,dx,dy,buttons,0`
    /// - Mouse button: `b,button,pressed`
    /// - Mouse wheel: `w,dx,dy`
    /// - Keyboard: `k,keysym,pressed`
    /// - Key down: `kd,keysym`
    /// - Key up: `ku,keysym`
    /// - Text input: `t,<utf8_text>`
    /// - Clipboard: `c,<base64_text>`
    /// - Ping: `p,timestamp`
    pub fn parse_input_text(text: &str) -> Result<InputEventData, WebRTCError> {
        let parts: Vec<&str> = text.split(',').collect();

        if parts.is_empty() {
            return Err(WebRTCError::DataChannelError("Empty input message".to_string()));
        }

        let mut event = InputEventData::default();

        match parts[0] {
            "m" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid mouse move format".to_string()));
                }
                event.event_type = InputEvent::MouseMove;
                event.mouse_x = parts[1].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse X".to_string()))?;
                event.mouse_y = parts[2].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse Y".to_string()))?;
                if parts.len() > 3 {
                    event.button_mask = parts[3].parse().unwrap_or(0);
                }
            }

            "m2" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid relative mouse move format".to_string()));
                }
                event.event_type = InputEvent::MouseMove;
                event.mouse_x = parts[1].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse dX".to_string()))?;
                event.mouse_y = parts[2].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid mouse dY".to_string()))?;
                event.text = "relative".to_string();
                if parts.len() > 3 {
                    event.button_mask = parts[3].parse().unwrap_or(0);
                }
            }

            "b" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid mouse button format".to_string()));
                }
                event.event_type = InputEvent::MouseButton;
                event.mouse_button = parts[1].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid button number".to_string()))?;
                event.button_pressed = parts[2] == "1";
            }

            "w" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid mouse wheel format".to_string()));
                }
                event.event_type = InputEvent::MouseWheel;
                event.wheel_delta_x = parts[1].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid wheel delta X".to_string()))?;
                event.wheel_delta_y = parts[2].parse()
                    .map_err(|_| WebRTCError::DataChannelError("Invalid wheel delta Y".to_string()))?;
            }

            "k" => {
                if parts.len() < 3 {
                    return Err(WebRTCError::DataChannelError("Invalid keyboard format".to_string()));
                }
                event.event_type = InputEvent::Keyboard;
                let keysym_str = parts[1];
                event.keysym = if keysym_str.starts_with("0x") || keysym_str.starts_with("0X") {
                    u32::from_str_radix(&keysym_str[2..], 16)
                        .map_err(|_| WebRTCError::DataChannelError("Invalid hex keysym".to_string()))?
                } else {
                    keysym_str.parse()
                        .map_err(|_| WebRTCError::DataChannelError("Invalid keysym".to_string()))?
                };
                event.key_pressed = parts[2] == "1";
            }

            "kd" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError("Invalid kd format".to_string()));
                }
                event.event_type = InputEvent::Keyboard;
                let keysym_str = parts[1];
                event.keysym = if keysym_str.starts_with("0x") || keysym_str.starts_with("0X") {
                    u32::from_str_radix(&keysym_str[2..], 16)
                        .map_err(|_| WebRTCError::DataChannelError("Invalid hex keysym".to_string()))?
                } else {
                    keysym_str.parse()
                        .map_err(|_| WebRTCError::DataChannelError("Invalid keysym".to_string()))?
                };
                event.key_pressed = true;
            }

            "ku" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError("Invalid ku format".to_string()));
                }
                event.event_type = InputEvent::Keyboard;
                let keysym_str = parts[1];
                event.keysym = if keysym_str.starts_with("0x") || keysym_str.starts_with("0X") {
                    u32::from_str_radix(&keysym_str[2..], 16)
                        .map_err(|_| WebRTCError::DataChannelError("Invalid hex keysym".to_string()))?
                } else {
                    keysym_str.parse()
                        .map_err(|_| WebRTCError::DataChannelError("Invalid keysym".to_string()))?
                };
                event.key_pressed = false;
            }

            "t" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError("Invalid text input format".to_string()));
                }
                event.event_type = InputEvent::TextInput;
                event.text = parts[1..].join(",");
            }

            "c" => {
                if parts.len() < 2 {
                    return Err(WebRTCError::DataChannelError("Invalid clipboard format".to_string()));
                }
                event.event_type = InputEvent::Clipboard;
                event.text = parts[1..].join(",");
            }

            "p" => {
                event.event_type = InputEvent::Ping;
                if parts.len() > 1 {
                    event.timestamp = parts[1].parse().unwrap_or(0);
                }
            }

            _ => {
                return Err(WebRTCError::DataChannelError(format!("Unknown input type: {}", parts[0])));
            }
        }

        Ok(event)
    }
}

/// Format an outgoing message for the DataChannel
pub fn format_output_message(msg_type: &str, data: &str) -> String {
    format!("{},{}", msg_type, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mouse_move() {
        let event = InputDataChannel::parse_input_text("m,100,200").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseMove);
        assert_eq!(event.mouse_x, 100);
        assert_eq!(event.mouse_y, 200);
    }

    #[test]
    fn test_parse_mouse_move_with_buttons() {
        let event = InputDataChannel::parse_input_text("m,100,200,3").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseMove);
        assert_eq!(event.button_mask, 3);
    }

    #[test]
    fn test_parse_mouse_button() {
        let event = InputDataChannel::parse_input_text("b,1,1").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseButton);
        assert_eq!(event.mouse_button, 1);
        assert!(event.button_pressed);
    }

    #[test]
    fn test_parse_keyboard() {
        let event = InputDataChannel::parse_input_text("k,0xff08,1").unwrap();
        assert_eq!(event.event_type, InputEvent::Keyboard);
        assert_eq!(event.keysym, 0xff08);
        assert!(event.key_pressed);
    }

    #[test]
    fn test_parse_keyboard_decimal() {
        let event = InputDataChannel::parse_input_text("k,65,1").unwrap();
        assert_eq!(event.event_type, InputEvent::Keyboard);
        assert_eq!(event.keysym, 65);
        assert!(event.key_pressed);
    }

    #[test]
    fn test_parse_wheel() {
        let event = InputDataChannel::parse_input_text("w,0,-120").unwrap();
        assert_eq!(event.event_type, InputEvent::MouseWheel);
        assert_eq!(event.wheel_delta_x, 0);
        assert_eq!(event.wheel_delta_y, -120);
    }

    #[test]
    fn test_parse_text_with_comma() {
        let event = InputDataChannel::parse_input_text("t,hello,world").unwrap();
        assert_eq!(event.event_type, InputEvent::TextInput);
        assert_eq!(event.text, "hello,world");
    }
}
